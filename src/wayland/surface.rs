use super::state::WaylandState;
use crate::{
	core::{delta::Delta, destroy_queue, registry::Registry},
	nodes::drawable::{model::ModelPart, shaders::PANEL_SHADER_BYTES},
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
	reexports::wayland_server::{self, protocol::wl_surface::WlSurface, DisplayHandle, Resource},
	wayland::compositor::{self, SurfaceData},
};
use std::{ffi::c_void, sync::Arc, time::Duration};
use stereokit::{
	Material, StereoKitDraw, Tex, TextureAddress, TextureFormat, TextureSample, TextureType,
	Transparency,
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
	pub dh: DisplayHandle,
	pub weak_surface: wayland_server::Weak<WlSurface>,
	mapped_data: Mutex<Option<CoreSurfaceData>>,
	sk_tex: OnceCell<Tex>,
	sk_mat: OnceCell<Arc<Material>>,
	material_offset: Mutex<Delta<u32>>,
	on_mapped: Box<dyn Fn() + Send + Sync>,
	on_commit: Box<dyn Fn(u32) + Send + Sync>,
	pub pending_material_applications: Registry<ModelPart>,
}

impl CoreSurface {
	pub fn add_to(
		dh: DisplayHandle,
		surface: &WlSurface,
		on_mapped: impl Fn() + Send + Sync + 'static,
		on_commit: impl Fn(u32) + Send + Sync + 'static,
	) {
		compositor::with_states(surface, |data| {
			data.data_map.insert_if_missing_threadsafe(|| {
				CORE_SURFACES.add(CoreSurface {
					dh,
					weak_surface: surface.downgrade(),
					mapped_data: Mutex::new(None),
					sk_tex: OnceCell::new(),
					sk_mat: OnceCell::new(),
					material_offset: Mutex::new(Delta::new(0)),
					on_mapped: Box::new(on_mapped) as Box<dyn Fn() + Send + Sync>,
					on_commit: Box::new(on_commit) as Box<dyn Fn(u32) + Send + Sync>,
					pending_material_applications: Registry::new(),
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

	pub fn process(&self, sk: &impl StereoKitDraw, renderer: &mut GlesRenderer) {
		let Some(wl_surface) = self.wl_surface() else {return};

		let sk_tex = self
			.sk_tex
			.get_or_init(|| sk.tex_create(TextureType::IMAGE_NO_MIPS, TextureFormat::RGBA32));
		self.sk_mat.get_or_init(|| {
			let shader = sk.shader_create_mem(&PANEL_SHADER_BYTES).unwrap();
			// let _ = renderer.with_context(|c| unsafe {
			// 	shader_inject(c, &mut shader, SIMULA_VERT_STR, SIMULA_FRAG_STR)
			// });

			let mat = sk.material_create(&shader);
			sk.material_set_texture(&mat, "diffuse", sk_tex.as_ref());
			sk.material_set_transparency(&mat, Transparency::Blend);
			Arc::new(mat)
		});

		// Let smithay handle buffer management (has to be done here as RendererSurfaceStates is not thread safe)
		on_commit_buffer_handler::<WaylandState>(&wl_surface);
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
		let just_mapped = mapped_data.is_none();
		self.with_states(|data| {
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
				sk.tex_set_surface(
					sk_tex.as_ref(),
					smithay_tex.tex_id() as usize as *mut c_void,
					TextureType::IMAGE_NO_MIPS,
					smithay::backend::renderer::gles::ffi::RGBA8.into(),
					smithay_tex.width() as i32,
					smithay_tex.height() as i32,
					1,
					false,
				);
				sk.tex_set_sample(sk_tex.as_ref(), TextureSample::Point);
				sk.tex_set_address(sk_tex.as_ref(), TextureAddress::Clamp);
			}
			if let Some(material_offset) = self.material_offset.lock().delta() {
				sk.material_set_queue_offset(sk_mat.as_ref().as_ref(), *material_offset as i32);
			}

			let surface_size = renderer_surface_state.surface_size().unwrap();
			let new_mapped_data = CoreSurfaceData {
				size: Vector2::from([surface_size.w as u32, surface_size.h as u32]),
				wl_tex: Some(SendWrapper::new(smithay_tex)),
			};
			*mapped_data = Some(new_mapped_data);
		});
		drop(mapped_data);
		if just_mapped {
			(self.on_mapped)();
		}
		self.apply_surface_materials();
	}

	pub fn frame(&self, sk: &impl StereoKitDraw, output: Output) {
		let Some(wl_surface) = self.wl_surface() else {return};

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

	pub fn apply_material(&self, model_part: &Arc<ModelPart>) {
		self.pending_material_applications.add_raw(model_part)
	}

	fn apply_surface_materials(&self) {
		for model_node in self.pending_material_applications.get_valid_contents() {
			model_node.replace_material(self.sk_mat.clone().get().unwrap().clone());
		}
		self.pending_material_applications.clear();
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
}
impl Drop for CoreSurface {
	fn drop(&mut self) {
		CORE_SURFACES.remove(self);

		destroy_queue::add(self.sk_tex.take());
		destroy_queue::add(self.sk_mat.take());
	}
}
