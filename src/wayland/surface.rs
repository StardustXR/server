use super::{shaders::PANEL_SHADER_BYTES, state::WaylandState};
use crate::{
	core::{destroy_queue, registry::Registry},
	nodes::drawable::model::Model,
};
use mint::Vector2;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use slog::Logger;
use smithay::{
	backend::renderer::{
		gles2::{Gles2Renderer, Gles2Texture},
		utils::{import_surface_tree, on_commit_buffer_handler, RendererSurfaceStateUserData},
		Texture,
	},
	desktop::utils::send_frames_surface_tree,
	output::Output,
	reexports::wayland_server::{
		self, protocol::wl_surface::WlSurface, Display, DisplayHandle, Resource,
	},
	wayland::compositor::{self, SurfaceData},
};
use std::{
	sync::{Arc, Weak},
	time::Duration,
};
use stereokit::{
	material::{Material, Transparency},
	shader::Shader,
	texture::{Texture as SKTexture, TextureAddress, TextureFormat, TextureSample, TextureType},
	StereoKit,
};

pub static CORE_SURFACES: Registry<CoreSurface> = Registry::new();

pub struct CoreSurfaceData {
	wl_tex: Option<SendWrapper<Gles2Texture>>,
	sk_tex: Option<SendWrapper<SKTexture>>,
	sk_mat: Option<Arc<SendWrapper<Material>>>,
	pub size: Vector2<u32>,
}
impl CoreSurfaceData {
	fn new(sk: &StereoKit) -> Self {
		let sk_tex = SendWrapper::new(
			SKTexture::create(sk, TextureType::ImageNoMips, TextureFormat::RGBA32).unwrap(),
		);
		let sk_mat = {
			let shader = Shader::from_mem(sk, PANEL_SHADER_BYTES).unwrap();
			let mat = Material::create(sk, &shader).unwrap();
			mat.set_parameter("diffuse", &*sk_tex);
			mat.set_transparency(Transparency::Blend);
			Arc::new(SendWrapper::new(mat))
		};
		CoreSurfaceData {
			wl_tex: None,
			sk_tex: Some(sk_tex),
			sk_mat: Some(sk_mat),
			size: Vector2::from([0, 0]),
		}
	}
	fn update_tex(&mut self, data: &RendererSurfaceStateUserData, renderer: &Gles2Renderer) {
		if let Some(surface_size) = data.borrow().surface_size() {
			self.size = Vector2::from([surface_size.w as u32, surface_size.h as u32]);
		}
		self.wl_tex = data
			.borrow()
			.texture(renderer)
			.cloned()
			.map(SendWrapper::new);
		if let Some(smithay_tex) = self.wl_tex.as_ref() {
			let sk_tex = self.sk_tex.as_ref().unwrap();
			unsafe {
				sk_tex.set_native(
					smithay_tex.tex_id() as usize,
					smithay::backend::renderer::gles2::ffi::RGBA8.into(),
					TextureType::Image,
					smithay_tex.width(),
					smithay_tex.height(),
					false,
				);
				sk_tex.set_sample(TextureSample::Point);
				sk_tex.set_address_mode(TextureAddress::Clamp);
			}
		}
	}
}
impl Drop for CoreSurfaceData {
	fn drop(&mut self) {
		destroy_queue::add(self.wl_tex.take());
		destroy_queue::add(self.sk_tex.take());
		destroy_queue::add(self.sk_mat.take());
	}
}

pub struct CoreSurface {
	display: Weak<Mutex<Display<WaylandState>>>,
	pub state: Weak<Mutex<WaylandState>>,
	pub dh: DisplayHandle,
	pub weak_surface: wayland_server::Weak<WlSurface>,
	pub mapped_data: Mutex<Option<CoreSurfaceData>>,
	pub pending_material_applications: Mutex<Vec<(Arc<Model>, u32)>>,
}

impl CoreSurface {
	pub fn new(
		state: &Arc<Mutex<WaylandState>>,
		display: &Arc<Mutex<Display<WaylandState>>>,
		dh: DisplayHandle,
		surface: &WlSurface,
	) -> Arc<Self> {
		CORE_SURFACES.add(CoreSurface {
			display: Arc::downgrade(display),
			state: Arc::downgrade(state),
			dh,
			weak_surface: surface.downgrade(),
			mapped_data: Mutex::new(None),
			pending_material_applications: Mutex::new(Vec::new()),
		})
	}

	pub fn process<F: FnOnce(&SurfaceData), M: FnOnce(&SurfaceData)>(
		&self,
		sk: &StereoKit,
		renderer: &mut Gles2Renderer,
		output: Output,
		time: Duration,
		log: &Logger,
		on_mapped: F,
		if_mapped: M,
	) {
		let Some(wl_surface) = self.wl_surface() else { return };

		// Let smithay handle buffer management (has to be done here as RendererSurfaceStates is not thread safe)
		on_commit_buffer_handler(&wl_surface);
		// Import all surface buffers into textures
		if import_surface_tree(renderer, &wl_surface, log).is_err() {
			return;
		}

		let mapped = compositor::with_states(&wl_surface, |data| {
			data.data_map
				.get::<RendererSurfaceStateUserData>()
				.map(|surface_states| surface_states.borrow().wl_buffer().is_some())
				.unwrap_or(false)
		});

		if !mapped {
			return;
		}

		let mut mapped_data = self.mapped_data.lock();
		let just_mapped = mapped_data.is_none();
		if just_mapped {
			*mapped_data = Some(CoreSurfaceData::new(sk));
		}
		drop(mapped_data);
		self.with_states(|data| {
			self.with_data(|mapped_data| {
				mapped_data.update_tex(
					data.data_map.get::<RendererSurfaceStateUserData>().unwrap(),
					renderer,
				);
			});
			self.apply_surface_materials();

			if just_mapped {
				on_mapped(data);
			}
			if_mapped(data);
		});

		send_frames_surface_tree(&wl_surface, &output, time, None, |_, _| {
			Some(output.clone())
		});
	}

	pub fn apply_material(&self, model: Arc<Model>, material_idx: u32) {
		self.pending_material_applications
			.lock()
			.push((model, material_idx));
	}

	fn apply_surface_materials(&self) {
		self.with_data(|mapped_data| {
			let mut pending_material_applications = self.pending_material_applications.lock();
			for (model, material_idx) in &*pending_material_applications {
				model
					.pending_material_replacements
					.lock()
					.insert(*material_idx, mapped_data.sk_mat.clone().unwrap());
			}
			pending_material_applications.clear();
		});
	}

	pub fn wayland_state(&self) -> Arc<Mutex<WaylandState>> {
		self.state.upgrade().unwrap()
	}

	pub fn wl_surface(&self) -> Option<WlSurface> {
		self.weak_surface.upgrade().ok()
	}

	pub fn with_states<F, T>(&self, f: F) -> Option<T>
	where
		F: FnOnce(&SurfaceData) -> T,
	{
		self.wl_surface()
			.map(|wl_surface| compositor::with_states(&wl_surface, f))
	}

	pub fn with_data<F, T>(&self, f: F) -> Option<T>
	where
		F: FnOnce(&mut CoreSurfaceData) -> T,
	{
		self.mapped_data.lock().as_mut().map(f)
	}

	pub fn flush_clients(&self) {
		self.display
			.upgrade()
			.unwrap()
			.lock()
			.flush_clients()
			.unwrap();
	}
}
impl Drop for CoreSurface {
	fn drop(&mut self) {
		CORE_SURFACES.remove(self);
	}
}
