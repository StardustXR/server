use super::{shaders::PANEL_SHADER_BYTES, state::WaylandState};
use crate::{
	core::{delta::Delta, destroy_queue, registry::Registry},
	nodes::drawable::model::Model,
};
use mint::Vector2;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use smithay::{
	backend::renderer::{
		gles::{GlesRenderer, GlesTexture},
		utils::{import_surface_tree, on_commit_buffer_handler, RendererSurfaceStateUserData},
		Renderer, Texture,
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
	lifecycle::StereoKitDraw,
	material::{Material, Transparency},
	shader::Shader,
	texture::{Texture as SKTexture, TextureAddress, TextureFormat, TextureSample, TextureType},
	time::StereoKitTime,
};

pub static CORE_SURFACES: Registry<CoreSurface> = Registry::new();

pub struct CoreSurfaceData {
	wl_tex: Option<SendWrapper<GlesTexture>>,
	pub size: Vector2<u32>,
}
impl Drop for CoreSurfaceData {
	fn drop(&mut self) {
		destroy_queue::add(self.wl_tex.take());
	}
}

pub struct CoreSurface {
	display: Weak<Mutex<Display<WaylandState>>>,
	pub dh: DisplayHandle,
	pub weak_surface: wayland_server::Weak<WlSurface>,
	mapped_data: Mutex<Option<CoreSurfaceData>>,
	sk_tex: OnceCell<SendWrapper<SKTexture>>,
	sk_mat: OnceCell<Arc<SendWrapper<Material>>>,
	material_offset: Mutex<Delta<u32>>,
	on_commit: Box<dyn Fn(u32) + Send + Sync>,
	pub pending_material_applications: Mutex<Vec<(Arc<Model>, u32)>>,
}

impl CoreSurface {
	pub fn add_to(
		display: &Arc<Mutex<Display<WaylandState>>>,
		dh: DisplayHandle,
		surface: &WlSurface,
		on_commit: impl Fn(u32) + Send + Sync + 'static,
	) {
		compositor::with_states(surface, |data| {
			data.data_map.insert_if_missing_threadsafe(|| {
				CORE_SURFACES.add(CoreSurface {
					display: Arc::downgrade(display),
					dh,
					weak_surface: surface.downgrade(),
					mapped_data: Mutex::new(None),
					sk_tex: OnceCell::new(),
					sk_mat: OnceCell::new(),
					material_offset: Mutex::new(Delta::new(0)),
					on_commit: Box::new(on_commit) as Box<dyn Fn(u32) + Send + Sync>,
					pending_material_applications: Mutex::new(Vec::new()),
				})
			});
		});
	}

	pub fn commit(&self, count: u32) {
		(self.on_commit)(count);
	}

	pub fn from_wl_surface(surf: &WlSurface) -> Option<Arc<CoreSurface>> {
		compositor::with_states(surf, |data| {
			data.data_map.get::<Arc<CoreSurface>>().cloned()
		})
	}

	pub fn process(&self, sk: &StereoKitDraw, renderer: &mut GlesRenderer) {
		let Some(wl_surface) = self.wl_surface() else { return };

		let sk_tex = self.sk_tex.get_or_init(|| {
			SendWrapper::new(
				SKTexture::create(sk, TextureType::ImageNoMips, TextureFormat::RGBA32).unwrap(),
			)
		});
		self.sk_mat.get_or_init(|| {
			let shader = Shader::from_mem(sk, PANEL_SHADER_BYTES).unwrap();
			let mat = Material::create(sk, &shader).unwrap();
			mat.set_parameter(sk, "diffuse", &**sk_tex);
			mat.set_transparency(sk, Transparency::Blend);
			Arc::new(SendWrapper::new(mat))
		});

		// Let smithay handle buffer management (has to be done here as RendererSurfaceStates is not thread safe)
		on_commit_buffer_handler(&wl_surface);
		// Import all surface buffers into textures
		if import_surface_tree(renderer, &wl_surface).is_err() {
			return;
		}

		let mapped = compositor::with_states(&wl_surface, |data| {
			data.data_map
				.get::<RendererSurfaceStateUserData>()
				.map(|surface_states| surface_states.borrow().buffer().is_some())
				.unwrap_or(false)
		});

		if !mapped {
			return;
		}

		let mut mapped_data = self.mapped_data.lock();
		self.with_states(|data| {
			// let just_mapped = mapped_data.is_none();
			// if just_mapped {
			let renderer_surface_state = data
				.data_map
				.get::<RendererSurfaceStateUserData>()
				.unwrap()
				.borrow();
			let smithay_tex = renderer_surface_state
				.texture::<GlesRenderer>(renderer.id())
				.unwrap()
				.clone();

			let sk_tex = self.sk_tex.get().unwrap();
			let sk_mat = self.sk_mat.get().unwrap();
			unsafe {
				sk_tex.set_native(
					smithay_tex.tex_id() as usize,
					smithay::backend::renderer::gles::ffi::RGBA8.into(),
					TextureType::ImageNoMips,
					smithay_tex.width(),
					smithay_tex.height(),
					false,
				);
				sk_tex.set_sample(TextureSample::Point);
				sk_tex.set_address_mode(TextureAddress::Clamp);
			}
			if let Some(material_offset) = self.material_offset.lock().delta() {
				sk_mat.set_queue_offset(sk, *material_offset as i32);
			}

			let surface_size = renderer_surface_state.surface_size().unwrap();
			let new_mapped_data = CoreSurfaceData {
				size: Vector2::from([surface_size.w as u32, surface_size.h as u32]),
				wl_tex: Some(SendWrapper::new(smithay_tex)),
			};
			*mapped_data = Some(new_mapped_data);
		});
		self.apply_surface_materials();
	}

	pub fn frame(&self, sk: &StereoKitDraw, output: Output) {
		let Some(wl_surface) = self.wl_surface() else { return };

		send_frames_surface_tree(
			&wl_surface,
			&output,
			Duration::from_secs_f64(sk.time_get()),
			None,
			|_, _| Some(output.clone()),
		);
	}

	pub fn set_material_offset(&self, material_offset: u32) {
		*self.material_offset.lock().value_mut() = material_offset;
	}

	pub fn apply_material(&self, model: Arc<Model>, material_idx: u32) {
		self.pending_material_applications
			.lock()
			.push((model, material_idx));
	}

	fn apply_surface_materials(&self) {
		for (model, material_idx) in self.pending_material_applications.lock().drain(0..) {
			model
				.pending_material_replacements
				.lock()
				.insert(material_idx, self.sk_mat.get().unwrap().clone());
		}
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

	pub fn size(&self) -> Option<Vector2<u32>> {
		self.mapped_data.lock().as_ref().map(|d| d.size)
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

		destroy_queue::add(self.sk_tex.take());
		destroy_queue::add(self.sk_mat.take());
	}
}
