use super::utils::WlSurfaceExt;
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
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use smithay::{
	backend::renderer::{
		Renderer, Texture,
		gles::{GlesRenderer, GlesTexture},
		utils::{RendererSurfaceStateUserData, import_surface_tree},
	},
	desktop::utils::send_frames_surface_tree,
	output::Output,
	reexports::wayland_server::{self, Resource, protocol::wl_surface::WlSurface},
};
use std::{
	ffi::c_void,
	sync::{Arc, OnceLock},
	time::Duration,
};
use stereokit_rust::{
	material::{Material, Transparency},
	shader::Shader,
	tex::{Tex, TexAddress, TexFormat, TexSample, TexType},
	util::Time,
};

pub static CORE_SURFACES: Registry<CoreSurface> = Registry::new();

pub struct CoreSurfaceData {
	wl_tex: Option<SendWrapper<GlesTexture>>,
}
impl Drop for CoreSurfaceData {
	fn drop(&mut self) {
		destroy_queue::add(self.wl_tex.take());
	}
}

pub struct CoreSurface {
	pub weak_surface: wayland_server::Weak<WlSurface>,
	mapped_data: Mutex<Option<CoreSurfaceData>>,
	sk_tex: OnceLock<Mutex<TexWrapper>>,
	sk_mat: OnceLock<Mutex<MaterialWrapper>>,
	material_offset: Mutex<Delta<u32>>,
	pub pending_material_applications: Registry<ModelPart>,
}

impl CoreSurface {
	pub fn add_to(surface: &WlSurface) {
		let core_surface = CORE_SURFACES.add(CoreSurface {
			weak_surface: surface.downgrade(),
			mapped_data: Mutex::new(None),
			sk_tex: OnceLock::new(),
			sk_mat: OnceLock::new(),
			material_offset: Mutex::new(Delta::new(0)),
			pending_material_applications: Registry::new(),
		});
		surface.insert_data(core_surface);
	}

	pub fn from_wl_surface(surf: &WlSurface) -> Option<Arc<CoreSurface>> {
		surf.get_data()
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

		self.update_textures(renderer);
		self.apply_surface_materials();
	}

	pub fn update_textures(&self, renderer: &mut GlesRenderer) {
		let Some(wl_surface) = self.wl_surface() else {
			return;
		};
		let mapped = wl_surface
			.get_data_raw::<RendererSurfaceStateUserData, _, _>(|surface_states| {
				surface_states.lock().unwrap().buffer().is_some()
			})
			.unwrap_or(false);

		if !mapped {
			return;
		}

		let mut mapped_data = self.mapped_data.lock();

		let Some(smithay_tex) = wl_surface
			.get_data_raw::<RendererSurfaceStateUserData, _, _>(|surface_states| {
				surface_states
					.lock()
					.unwrap()
					.texture::<GlesRenderer>(renderer.id())
					.cloned()
			})
			.flatten()
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

		let new_mapped_data = CoreSurfaceData {
			wl_tex: Some(SendWrapper::new(smithay_tex)),
		};
		*mapped_data = Some(new_mapped_data);
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
}
impl Drop for CoreSurface {
	fn drop(&mut self) {
		CORE_SURFACES.remove(self);

		destroy_queue::add(self.sk_tex.take());
		destroy_queue::add(self.sk_mat.take());
	}
}
