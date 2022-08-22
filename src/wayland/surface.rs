use std::{cell::RefCell, fmt::Error};

use glam::vec2;
use once_cell::sync::OnceCell;
use send_wrapper::SendWrapper;
use smithay::backend::renderer::{gles2::Gles2Texture, Texture};
use stereokit::{
	material::Material,
	shader::Shader,
	texture::{Texture as SKTexture, TextureAddress, TextureFormat, TextureSample, TextureType},
	StereoKit,
};

use super::shaders::SIMULA_SHADER_BYTES;

pub struct CoreSurface {
	pub(crate) wl_tex: RefCell<Option<SendWrapper<Gles2Texture>>>,
	sk_tex: OnceCell<SKTexture>,
	sk_mat: OnceCell<Material>,
}

impl CoreSurface {
	pub fn new() -> Self {
		CoreSurface {
			wl_tex: RefCell::new(None),
			sk_tex: OnceCell::new(),
			sk_mat: OnceCell::new(),
		}
	}

	pub fn update_tex(&self, sk: &StereoKit) {
		let sk_tex = self
			.sk_tex
			.get_or_try_init(|| {
				SKTexture::create(sk, TextureType::ImageNoMips, TextureFormat::RGBA32).ok_or(Error)
			})
			.unwrap();
		let sk_mat = self
			.sk_mat
			.get_or_try_init(|| {
				let shader = Shader::from_mem(sk, SIMULA_SHADER_BYTES).unwrap();
				Material::create(sk, &shader).ok_or(Error).map(|mat| {
					mat.set_parameter("diffuse", self.sk_tex.get().unwrap());
					mat
				})
			})
			.unwrap();
		if let Some(smithay_tex) = self.wl_tex.borrow().as_ref() {
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
