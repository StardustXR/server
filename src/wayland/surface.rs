use super::utils;
use crate::{
	core::{delta::Delta, destroy_queue, registry::Registry},
	nodes::{
		drawable::{
			model::{MaterialWrapper, ModelPart},
			shaders::PANEL_SHADER_BYTES,
		},
		items::camera::TexWrapper,
	},
};
use mint::Vector2;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use smithay::{
	backend::renderer::{
		gles::{GlesRenderer, GlesTexture},
		utils::{import_surface_tree, RendererSurfaceStateUserData},
		Renderer, Texture,
	},
	desktop::utils::send_frames_surface_tree,
	output::Output,
	reexports::wayland_server::{self, protocol::wl_surface::WlSurface, Resource},
	wayland::compositor::{self, SurfaceData},
};
use std::{ffi::c_void, sync::Arc, time::Duration};
use stereokit_rust::{
	material::{Material, Transparency},
	shader::Shader,
	tex::{Tex, TexAddress, TexFormat, TexSample, TexType},
	util::Time,
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
	pub weak_surface: wayland_server::Weak<WlSurface>,
	mapped_data: Mutex<Option<CoreSurfaceData>>,
	sk_tex: OnceCell<Mutex<TexWrapper>>,
	sk_mat: OnceCell<Mutex<MaterialWrapper>>,
	material_offset: Mutex<Delta<u32>>,
	on_mapped: Mutex<Box<dyn Fn() + Send + Sync>>,
	on_commit: Mutex<Box<dyn Fn(u32) + Send + Sync>>,
	pub pending_material_applications: Registry<ModelPart>,
}

impl CoreSurface {
	pub fn add_to(
		surface: &WlSurface,
		on_mapped: impl Fn() + Send + Sync + 'static,
		on_commit: impl Fn(u32) + Send + Sync + 'static,
	) {
		let core_surface = CORE_SURFACES.add(CoreSurface {
			weak_surface: surface.downgrade(),
			mapped_data: Mutex::new(None),
			sk_tex: OnceCell::new(),
			sk_mat: OnceCell::new(),
			material_offset: Mutex::new(Delta::new(0)),
			on_mapped: Mutex::new(Box::new(on_mapped) as Box<dyn Fn() + Send + Sync>),
			on_commit: Mutex::new(Box::new(on_commit) as Box<dyn Fn(u32) + Send + Sync>),
			pending_material_applications: Registry::new(),
		});
		utils::insert_data_raw(surface, core_surface);
	}

	pub fn commit(&self, count: u32) {
		(*self.on_commit.lock())(count);
	}

	pub fn from_wl_surface(surf: &WlSurface) -> Option<Arc<CoreSurface>> {
		utils::get_data(surf)
	}

	pub fn decycle(&self) {
		*self.on_mapped.lock() = Box::new(|| {});
		*self.on_commit.lock() = Box::new(|_| {});
	}

	pub fn process(&self, renderer: &mut GlesRenderer) {
		let Some(wl_surface) = self.wl_surface() else {
			return;
		};

		let sk_tex = self.sk_tex.get_or_init(|| {
			Mutex::new(TexWrapper(Tex::new(
				TexType::ImageNomips,
				TexFormat::RGBA32Linear,
				nanoid::nanoid!(),
			)))
		});
		self.sk_mat.get_or_init(|| {
			let shader = Shader::from_memory(PANEL_SHADER_BYTES).unwrap();
			// let _ = renderer.with_context(|c| unsafe {
			// 	shader_inject(c, &mut shader, SIMULA_VERT_STR, SIMULA_FRAG_STR)
			// });

			let mut mat = Material::new(shader, None);
			mat.diffuse_tex(&sk_tex.lock().0);
			mat.transparency(Transparency::Blend);
			Mutex::new(MaterialWrapper(mat))
		});

		// Import all surface buffers into textures
		if import_surface_tree(renderer, &wl_surface).is_err() {
			return;
		}

		let mapped = compositor::with_states(&wl_surface, |data| {
			data.data_map
				.get::<RendererSurfaceStateUserData>()
				.map(|surface_states| surface_states.lock().unwrap().buffer().is_some())
				.unwrap_or(false)
		});

		if !mapped {
			return;
		}

		let mut mapped_data = self.mapped_data.lock();
		let just_mapped = mapped_data.is_none();
		self.with_states(|data| {
			let Some(renderer_surface_state) = data
				.data_map
				.get::<RendererSurfaceStateUserData>()
				.map(std::sync::Mutex::lock)
				.map(Result::ok)
				.flatten()
			else {
				return;
			};
			let Some(smithay_tex) = renderer_surface_state
				.texture::<GlesRenderer>(renderer.id())
				.cloned()
			else {
				return;
			};

			let Some(sk_tex) = self.sk_tex.get() else {
				return;
			};
			let Some(sk_mat) = self.sk_mat.get() else {
				return;
			};
			sk_tex
				.lock()
				.0
				.set_native_surface(
					smithay_tex.tex_id() as usize as *mut c_void,
					TexType::ImageNomips,
					smithay::backend::renderer::gles::ffi::RGBA8.into(),
					smithay_tex.width() as i32,
					smithay_tex.height() as i32,
					1,
					false,
				)
				.sample_mode(TexSample::Point)
				.address_mode(TexAddress::Clamp);

			if let Some(material_offset) = self.material_offset.lock().delta() {
				sk_mat.lock().0.queue_offset(*material_offset as i32);
			}

			let Some(surface_size) = renderer_surface_state.surface_size() else {
				return;
			};
			let new_mapped_data = CoreSurfaceData {
				size: Vector2::from([surface_size.w as u32, surface_size.h as u32]),
				wl_tex: Some(SendWrapper::new(smithay_tex)),
			};
			*mapped_data = Some(new_mapped_data);
		});
		drop(mapped_data);
		if just_mapped {
			(*self.on_mapped.lock())();
		}
		self.apply_surface_materials();
	}

	pub fn frame(&self, output: Output) {
		let Some(wl_surface) = self.wl_surface() else {
			return;
		};

		send_frames_surface_tree(
			&wl_surface,
			&output,
			Duration::from_secs_f64(Time::get_total_unscaled()),
			None,
			|_, _| Some(output.clone()),
		);
	}

	pub fn set_material_offset(&self, material_offset: u32) {
		*self.material_offset.lock().value_mut() = material_offset;
	}

	pub fn apply_material(&self, model_part: &Arc<ModelPart>) {
		self.pending_material_applications.add_raw(model_part)
	}

	fn apply_surface_materials(&self) {
		if let Some(sk_mat) = self.sk_mat.get() {
			let sk_mat = sk_mat.lock();
			for model_node in self.pending_material_applications.get_valid_contents() {
				model_node.replace_material_now(&sk_mat.0);
			}
			self.pending_material_applications.clear();
		}
	}

	pub fn wl_surface(&self) -> Option<WlSurface> {
		self.weak_surface.upgrade().ok()
	}

	pub fn with_states<T, F: FnOnce(&SurfaceData) -> T>(&self, f: F) -> Option<T> {
		self.wl_surface()
			.map(|wl_surface| compositor::with_states(&wl_surface, f))
	}

	pub fn size(&self) -> Option<Vector2<u32>> {
		self.mapped_data.lock().as_ref().map(|d| d.size)
	}
}
impl Drop for CoreSurface {
	fn drop(&mut self) {
		CORE_SURFACES.remove(self);

		destroy_queue::add(self.sk_tex.take());
		destroy_queue::add(self.sk_mat.take());
	}
}
