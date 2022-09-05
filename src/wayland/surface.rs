use std::{fmt::Error, mem, sync::Arc};

use glam::vec2;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use smithay::{
	backend::renderer::{
		gles2::{Gles2Renderer, Gles2Texture},
		utils::RendererSurfaceStateUserData,
		Texture,
	},
	wayland::compositor::SurfaceData,
};
use stereokit::{
	material::{Material, Transparency},
	shader::Shader,
	texture::{Texture as SKTexture, TextureAddress, TextureFormat, TextureSample, TextureType},
	StereoKit,
};

use crate::core::destroy_queue;

use super::shaders::PANEL_SHADER_BYTES;

pub struct CoreSurface {
	pub wl_tex: Mutex<Option<SendWrapper<Gles2Texture>>>,
	sk_tex: OnceCell<SendWrapper<SKTexture>>,
	pub sk_mat: OnceCell<Arc<SendWrapper<Material>>>,
}

impl CoreSurface {
	pub fn new() -> Self {
		CoreSurface {
			wl_tex: Mutex::new(None),
			sk_tex: OnceCell::new(),
			sk_mat: OnceCell::new(),
		}
	}

	pub fn update_tex(&self, sk: &StereoKit, data: &SurfaceData, renderer: &Gles2Renderer) {
		let sk_tex = self
			.sk_tex
			.get_or_try_init(|| {
				SKTexture::create(sk, TextureType::ImageNoMips, TextureFormat::RGBA32)
					.ok_or(Error)
					.map(SendWrapper::new)
			})
			.unwrap();
		let sk_mat = self
			.sk_mat
			.get_or_try_init(|| {
				let shader = Shader::from_mem(sk, PANEL_SHADER_BYTES).unwrap();
				Material::create(sk, &shader)
					.ok_or(Error)
					.map(|mat| {
						mat.set_parameter("diffuse", &**self.sk_tex.get().unwrap());
						mat.set_transparency(Transparency::Blend);
						mat
					})
					.map(|mat| Arc::new(SendWrapper::new(mat)))
			})
			.unwrap();

		if let Some(surface_states) = data.data_map.get::<RendererSurfaceStateUserData>() {
			*self.wl_tex.lock() = surface_states
				.borrow()
				.texture(renderer)
				.cloned()
				.map(SendWrapper::new);
		}
		if let Some(smithay_tex) = self.wl_tex.lock().as_ref() {
			unsafe {
				sk_tex.set_native(
					smithay_tex.tex_id() as usize,
					smithay::backend::renderer::gles2::ffi::RGBA8.into(),
					TextureType::Image,
					smithay_tex.width(),
					smithay_tex.height(),
					false,
				);
				let size: mint::Vector2<f32> =
					vec2(smithay_tex.width() as f32, smithay_tex.height() as f32).into();
				sk_mat.set_parameter("size", &size);
				sk_tex.set_sample(TextureSample::Point);
				sk_tex.set_address_mode(TextureAddress::Clamp);
			}
		}
	}
}
impl Drop for CoreSurface {
	fn drop(&mut self) {
		destroy_queue::add(mem::replace(self.wl_tex.get_mut(), None));
		self.sk_tex.take().map(destroy_queue::add);
		self.sk_mat.take().map(destroy_queue::add);
	}
}
