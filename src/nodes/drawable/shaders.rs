use std::{ffi::CString, mem::transmute};

use smithay::backend::renderer::gles::ffi::{Gles2, FRAGMENT_SHADER, VERTEX_SHADER};
use stereokit::Shader;

struct FfiAssetHeader {
	asset_type: i32,
	asset_state: i32,
	id: u64,
	index: u64,
	refs: i32,
	debug: *mut u8,
}

struct FfiSkgShader {
	meta: *mut u8,
	vertex: u32,
	pixel: u32,
	program: u32,
	compute: u32,
}

struct FfiShader {
	header: FfiAssetHeader,
	shader: FfiSkgShader,
}

unsafe fn load_shader(c: &Gles2, source: &str, stage: u32) -> u32 {
	let shader = c.CreateShader(stage);
	let shader_source = CString::new(source).unwrap();
	c.ShaderSource(shader, 1, &shader_source.as_ptr(), std::ptr::null());
	c.CompileShader(shader);
	shader
}

unsafe fn link_program(c: &Gles2, vert: u32, frag: u32) -> u32 {
	let program = c.CreateProgram();
	c.AttachShader(program, vert);
	c.AttachShader(program, frag);
	c.LinkProgram(program);
	program
}

pub unsafe fn shader_inject(c: &Gles2, sk_shader: &mut Shader, vert_str: &str, frag_str: &str) {
	let gl_vert = dbg!(load_shader(c, vert_str, VERTEX_SHADER));
	let gl_frag = dbg!(load_shader(c, frag_str, FRAGMENT_SHADER));
	let gl_prog = dbg!(link_program(c, gl_vert, gl_frag));

	let shader: *mut FfiShader = transmute(sk_shader.0.as_mut());
	if let Some(shader) = shader.as_mut() {
		shader.shader.vertex = gl_vert;
		shader.shader.pixel = gl_frag;
		shader.shader.program = gl_prog;
	}
}
