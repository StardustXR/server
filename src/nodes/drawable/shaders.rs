#![allow(dead_code)]
use smithay::backend::renderer::gles::{
	ffi::{self, Gles2, FRAGMENT_SHADER, VERTEX_SHADER},
	GlesError,
};
use stereokit_rust::shader::{Shader, _ShaderT};
use tracing::error;

// Simula shader with fancy lanzcos sampling
pub const UNLIT_SHADER_BYTES: &[u8] = include_bytes!("assets/shaders/shader_unlit_gamma.hlsl.sks");

// Simula shader with fancy lanzcos sampling
pub const PANEL_SHADER_BYTES: &[u8] = include_bytes!("assets/shaders/shader_unlit_simula.hlsl.sks");

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

unsafe fn compile_shader(
	gl: &ffi::Gles2,
	variant: ffi::types::GLuint,
	src: &str,
) -> Result<ffi::types::GLuint, GlesError> {
	let shader = gl.CreateShader(variant);
	if shader == 0 {
		return Err(GlesError::CreateShaderObject);
	}

	gl.ShaderSource(
		shader,
		1,
		&src.as_ptr() as *const *const u8 as *const *const ffi::types::GLchar,
		&(src.len() as i32) as *const _,
	);
	gl.CompileShader(shader);

	let mut status = ffi::FALSE as i32;
	gl.GetShaderiv(shader, ffi::COMPILE_STATUS, &mut status as *mut _);
	if status == ffi::FALSE as i32 {
		let mut max_len = 0;
		gl.GetShaderiv(shader, ffi::INFO_LOG_LENGTH, &mut max_len as *mut _);

		let mut error = Vec::with_capacity(max_len as usize);
		let mut len = 0;
		gl.GetShaderInfoLog(
			shader,
			max_len as _,
			&mut len as *mut _,
			error.as_mut_ptr() as *mut _,
		);
		error.set_len(len as usize);

		error!(
			"[GL] {}",
			std::str::from_utf8(&error).unwrap_or("<Error Message no utf8>")
		);

		gl.DeleteShader(shader);
		return Err(GlesError::ShaderCompileError);
	}

	Ok(shader)
}

unsafe fn link_program(
	gl: &ffi::Gles2,
	vert: ffi::types::GLuint,
	frag: ffi::types::GLuint,
) -> Result<ffi::types::GLuint, GlesError> {
	let program = gl.CreateProgram();
	gl.AttachShader(program, vert);
	gl.AttachShader(program, frag);
	gl.LinkProgram(program);
	// gl.DetachShader(program, vert);
	// gl.DetachShader(program, frag);
	// gl.DeleteShader(vert);
	// gl.DeleteShader(frag);

	let mut status = ffi::FALSE as i32;
	gl.GetProgramiv(program, ffi::LINK_STATUS, &mut status as *mut _);
	if status == ffi::FALSE as i32 {
		let mut max_len = 0;
		gl.GetProgramiv(program, ffi::INFO_LOG_LENGTH, &mut max_len as *mut _);

		let mut error = Vec::with_capacity(max_len as usize);
		let mut len = 0;
		gl.GetProgramInfoLog(
			program,
			max_len as _,
			&mut len as *mut _,
			error.as_mut_ptr() as *mut _,
		);
		error.set_len(len as usize);

		error!(
			"[GL] {}",
			std::str::from_utf8(&error).unwrap_or("<Error Message no utf8>")
		);

		gl.DeleteProgram(program);
		return Err(GlesError::ProgramLinkError);
	}

	Ok(program)
}

pub unsafe fn shader_inject(
	c: &Gles2,
	sk_shader: &mut Shader,
	vert_str: &str,
	frag_str: &str,
) -> Result<(), GlesError> {
	let gl_vert = compile_shader(c, VERTEX_SHADER, vert_str)?;
	let gl_frag = compile_shader(c, FRAGMENT_SHADER, frag_str)?;
	let gl_prog = link_program(c, gl_vert, gl_frag)?;

	let shader = sk_shader.0.as_mut() as *mut _ShaderT as *mut FfiShader;
	if let Some(shader) = shader.as_mut() {
		shader.shader.vertex = gl_vert;
		shader.shader.pixel = gl_frag;
		shader.shader.program = gl_prog;
	}

	Ok(())
}
