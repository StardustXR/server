use super::{buffer::Buffer, callback::Callback};
use crate::{
	BevyMaterial,
	core::registry::Registry,
	nodes::{drawable::model::ModelPart, items::panel::Geometry},
	wayland::{
		Message, MessageSink,
		util::{ClientExt, DoubleBuffer},
		xdg::{popup::Popup, toplevel::Toplevel},
	},
};
use bevy::{
	asset::{Assets, Handle},
	image::Image,
	render::alpha::AlphaMode,
};
use bevy_dmabuf::import::ImportedDmatexs;
use mint::Vector2;
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock, atomic::Ordering};
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::core::wayland::{wl_output::Transform, wl_surface::*},
	},
	wire::ObjectId,
};

pub static WL_SURFACE_REGISTRY: Registry<Surface> = Registry::new();

#[derive(Debug, Clone)]
pub enum SurfaceRole {
	XdgToplevel(Arc<Toplevel>),
	XDGPopup(Arc<Popup>),
}

#[derive(Debug, Clone)]
pub struct SurfaceState {
	pub buffer: Option<Arc<Buffer>>,
	pub density: f32,
	pub geometry: Option<Geometry>,
	pub min_size: Option<Vector2<u32>>,
	pub max_size: Option<Vector2<u32>>,
	clean_lock: OnceLock<()>,
}
impl Default for SurfaceState {
	fn default() -> Self {
		Self {
			buffer: Default::default(),
			density: 1.0,
			geometry: None,
			min_size: None,
			max_size: None,
			clean_lock: Default::default(),
		}
	}
}

// if returning false, don't run this callback again... just remove it
pub type OnCommitCallback = Box<dyn Fn(&Surface, &SurfaceState) -> bool + Send + Sync>;

#[derive(Dispatcher)]
pub struct Surface {
	pub id: ObjectId,
	state: Mutex<DoubleBuffer<SurfaceState>>,
	pub message_sink: MessageSink,
	pub role: Mutex<Option<SurfaceRole>>,
	frame_callback_object: Mutex<Option<Arc<Callback>>>,
	on_commit_handlers: Mutex<Vec<OnCommitCallback>>,
	material: OnceLock<Handle<BevyMaterial>>,
	pending_material_applications: Registry<ModelPart>,
}
impl std::fmt::Debug for Surface {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Surface")
			.field("state", &self.state)
			.field("message_sink", &self.message_sink)
			.field("role", &self.role)
			.field("frame_callback_object", &self.frame_callback_object)
			.field(
				"on_commit_handlers",
				&format!("<{} handlers>", self.on_commit_handlers.lock().len()),
			)
			.finish()
	}
}
impl Surface {
	pub fn new(client: &Client, id: ObjectId) -> Self {
		Surface {
			id,
			state: Default::default(),
			message_sink: client.message_sink(),
			role: Mutex::new(None),
			frame_callback_object: Default::default(),
			on_commit_handlers: Mutex::new(Vec::new()),
			material: OnceLock::new(),
			pending_material_applications: Registry::new(),
		}
	}

	pub fn pending_state(&self) -> parking_lot::MutexGuard<'_, DoubleBuffer<SurfaceState>> {
		self.state.lock()
	}

	pub fn add_commit_handler<F: Fn(&Surface, &SurfaceState) -> bool + Send + Sync + 'static>(
		&self,
		handler: F,
	) {
		let mut handlers = self.on_commit_handlers.lock();
		handlers.push(Box::new(handler));
	}

	pub fn update_graphics(
		&self,
		dmatexes: &ImportedDmatexs,
		materials: &mut Assets<BevyMaterial>,
		images: &mut Assets<Image>,
	) {
		let state_lock = self.state.lock();
		if state_lock.current().clean_lock.get().is_some() {
			// then we don't need to reupload the texture
			return;
		}
		let Some(buffer) = state_lock.current().buffer.clone() else {
			return;
		};

		let material = self.material.get_or_init(|| {
			// // Set default shader parameters
			// let mut params = mat_wrapper.0.get_all_param_info();
			// params.set_vec2("uv_scale", stereokit_rust::maths::Vec2::new(1.0, 1.0));
			// params.set_vec2("uv_offset", stereokit_rust::maths::Vec2::new(0.0, 0.0));
			// params.set_float("fcFactor", 1.0);
			// params.set_float("ripple", 4.0);
			// params.set_float("alpha_min", 0.0);
			// params.set_float("alpha_max", 1.0);

			materials.add(BevyMaterial {
				unlit: true,
				..Default::default()
			})
		});

		if let Some(new_tex) = buffer.update_tex(dmatexes, images) {
			buffer.rendered.store(true, Ordering::Relaxed);
			let material = materials.get_mut(material).unwrap();
			material.base_color_texture.replace(new_tex);
			material.alpha_mode = if buffer.is_transparent() {
				AlphaMode::Premultiplied
			} else {
				AlphaMode::Opaque
			};
		}

		self.apply_surface_materials();
		let _ = state_lock.current().clean_lock.set(());
	}

	pub fn apply_material(&self, model_part: &Arc<ModelPart>) {
		// tracing::info!("uwu applying material");
		self.pending_material_applications.add_raw(model_part)
	}

	fn apply_surface_materials(&self) {
		let Some(mat) = self.material.get() else {
			return;
		};

		for model_node in self.pending_material_applications.get_valid_contents() {
			model_node.replace_material(mat.clone());
		}
		self.pending_material_applications.clear();
	}
	fn mark_dirty(&self) {
		self.state.lock().pending.clean_lock = Default::default();
	}
	pub fn current_state(&self) -> SurfaceState {
		self.state.lock().current().clone()
	}
	pub fn frame_event(&self) {
		if let Some(callback_obj) = self.frame_callback_object.lock().take() {
			let _ = self.message_sink.send(Message::Frame(callback_obj));
		}
	}
	// pub fn size(&self) -> Option<Vector2<u32>> {
	// 	self.state
	// 		.lock()
	// 		.current()
	// 		.buffer
	// 		.as_ref()
	// 		.map(|b| [b.size.x as u32, b.size.y as u32].into())
	// }

	// pub async fn release_old_buffer(&self, client: &mut Client) -> Result<()> {
	// 	let (old_buffer, object) = {
	// 		let lock = self.state.lock();

	// 		let Some(old_buffer) = lock.current().buffer.clone() else {
	// 			return Ok(());
	// 		};
	// 		let new_buffer = lock.pending.buffer.as_ref();
	// 		if new_buffer.map(Arc::as_ptr) == Some(Arc::as_ptr(&old_buffer)) {
	// 			return Ok(());
	// 		}
	// 		drop(lock);

	// 		(old_buffer.clone(), old_buffer.id)
	// 	};
	// 	old_buffer.release(client, object).await?;

	// 	Ok(())
	// }
}
impl WlSurface for Surface {
	/// https://wayland.app/protocols/wayland#wl_surface:request:attach
	async fn attach(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		buffer: Option<ObjectId>,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		self.state.lock().pending.buffer = buffer.and_then(|b| client.get::<Buffer>(b));
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:damage
	async fn damage(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		// should be more intelligent about this but for now just make it copy everything to gpu next frame again
		self.mark_dirty();
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:frame
	async fn frame(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		callback_id: ObjectId,
	) -> Result<()> {
		let callback = client.insert(callback_id, Callback(callback_id));
		self.frame_callback_object.lock().replace(callback);
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_opaque_region
	async fn set_opaque_region(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_region: Option<ObjectId>,
	) -> Result<()> {
		// nothing we can really do to repaint behind this so ignore it
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_input_region
	async fn set_input_region(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_region: Option<ObjectId>,
	) -> Result<()> {
		// too complicated to implement this for now so who the hell cares
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:commit
	async fn commit(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		{
			let mut lock = self.state.lock();

			// If we're getting a new buffer and the current one is DMA-BUF, release it
			if let Some(new_buffer) = &lock.pending.buffer {
				if let Some(current_buffer) = &lock.current().buffer {
					// Don't release if it's the same buffer being reused
					if !Arc::ptr_eq(new_buffer, current_buffer)
						&& !current_buffer.can_release_after_update()
					{
						let _ = self
							.message_sink
							.send(Message::ReleaseBuffer(current_buffer.clone()));
					}
				}
			}

			let dirty = lock.current().clean_lock.get().is_none()
				|| lock.pending.clean_lock.take().is_none();
			lock.apply();

			if !dirty {
				let _ = lock.current().clean_lock.set(());
			}
		}

		let current_state = self.current_state();
		let mut handlers = self.on_commit_handlers.lock();
		handlers.retain(|f| (f)(self, &current_state));
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_buffer_transform
	async fn set_buffer_transform(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_transform: Transform,
	) -> Result<()> {
		// we just don't have the output transform or fullscreen at all so this optimization is never needed
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_buffer_scale
	async fn set_buffer_scale(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		scale: i32,
	) -> Result<()> {
		self.state.lock().pending.density = scale as f32;
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:damage_buffer
	async fn damage_buffer(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		// we should upload only chunks to the gpu and do subimage copy but that's a lot rn so we won't
		self.mark_dirty();
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:offset
	async fn offset(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:destroy
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

impl Drop for Surface {
	fn drop(&mut self) {
		self.role.lock().take();
	}
}
