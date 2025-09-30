use super::{buffer::Buffer, callback::Callback};
use crate::{
	BevyMaterial,
	core::registry::Registry,
	nodes::{
		drawable::model::ModelPart,
		items::panel::{Geometry, PanelItem, SurfaceId},
	},
	wayland::{
		Client, Message, MessageSink, WaylandError, WaylandResult,
		core::buffer::BufferUsage,
		presentation::{MonotonicTimestamp, PresentationFeedback},
		util::{ClientExt, DoubleBuffer},
		xdg::backend::XdgBackend,
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
use waynest_server::Client as _;
use std::{
	fmt::Display,
	sync::{Arc, OnceLock, Weak},
};
use waynest::ObjectId;
use waynest_protocols::server::{
	core::wayland::{wl_output::Transform, wl_surface::*},
	stable::presentation_time::wp_presentation_feedback::{Kind, WpPresentationFeedback},
};

pub static WL_SURFACE_REGISTRY: Registry<Surface> = Registry::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceRole {
	Cursor,
	Subsurface,
	XdgToplevel,
	XdgPopup,
}
impl Display for SurfaceRole {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SurfaceRole::Cursor => f.write_str("SurfaceRole::Cursor"),
			SurfaceRole::Subsurface => f.write_str("SurfaceRole::Subsurface"),
			SurfaceRole::XdgToplevel => f.write_str("SurfaceRole::XdgToplevel"),
			SurfaceRole::XdgPopup => f.write_str("SurfaceRole::XdgPopup"),
		}
	}
}

#[derive(Debug, Clone)]
pub struct BufferState {
	pub buffer: Arc<Buffer>,
	pub usage: Option<Arc<BufferUsage>>,
}

#[derive(Debug, Clone)]
pub struct SurfaceState {
	pub buffer: Option<BufferState>,
	pub density: f32,
	pub geometry: Option<Geometry>,
	pub min_size: Option<Vector2<u32>>,
	pub max_size: Option<Vector2<u32>>,
	frame_callbacks: Vec<Arc<Callback>>,
}
impl Default for SurfaceState {
	fn default() -> Self {
		Self {
			buffer: Default::default(),
			density: 1.0,
			geometry: None,
			min_size: None,
			max_size: None,
			frame_callbacks: Vec::new(),
		}
	}
}
impl SurfaceState {
	pub fn has_valid_buffer(&self) -> bool {
		self.buffer
			.as_ref()
			.is_some_and(|b| b.buffer.size().x > 0 && b.buffer.size().y > 0)
	}
}

// if returning false, don't run this callback again... just remove it
pub type OnCommitCallback = Box<dyn FnMut(&Surface, &SurfaceState) -> bool + Send + Sync>;
#[derive(waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Surface {
	pub id: ObjectId,
	pub surface_id: OnceLock<SurfaceId>,
	state: Mutex<DoubleBuffer<SurfaceState>>,
	pub message_sink: MessageSink,
	pub role: OnceLock<SurfaceRole>,
	pub panel_item: Mutex<Weak<PanelItem<XdgBackend>>>,
	on_commit_handlers: Mutex<Vec<OnCommitCallback>>,
	frame_callbacks: Mutex<Vec<Arc<Callback>>>,
	material: OnceLock<Handle<BevyMaterial>>,
	pending_material_applications: Registry<ModelPart>,
	presentation_feedback: Mutex<Vec<Arc<PresentationFeedback>>>,
}
impl std::fmt::Debug for Surface {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Surface")
			.field("id", &self.id)
			.field("surface_id", &self.surface_id)
			.field("state", &self.state)
			.field("message_sink", &self.message_sink)
			.field("role", &self.role)
			.field(
				"on_commit_handlers",
				&format!("<{} handlers>", self.on_commit_handlers.lock().len()),
			)
			.field("material", &self.material)
			.field("presentation_feedback", &self.presentation_feedback)
			.finish()
	}
}
impl Surface {
	#[tracing::instrument(level = "debug", skip_all)]
	pub fn new(client: &Client, id: ObjectId) -> Self {
		Surface {
			id,
			surface_id: OnceLock::new(),
			state: Default::default(),
			message_sink: client.message_sink(),
			role: OnceLock::new(),
			panel_item: Mutex::new(Weak::default()),
			on_commit_handlers: Mutex::new(Vec::new()),
			frame_callbacks: Mutex::new(Vec::new()),
			material: OnceLock::new(),
			pending_material_applications: Registry::new(),
			presentation_feedback: Mutex::default(),
		}
	}

	pub async fn try_set_role(
		&self,
		role: SurfaceRole,
		role_error: impl Into<u32>,
	) -> WaylandResult<()> {
		match self.role.get().cloned() {
			Some(current_role) => {
				if current_role == role {
					Ok(())
				} else {
					Err(WaylandError::Fatal {
						object_id: self.id,
						code: role_error.into(),
						message: "Surface has an incomparible role",
					})
				}
			}
			None => {
				let _ = self.role.set(role);
				Ok(())
			}
		}
	}

	#[tracing::instrument(level = "debug", skip_all)]
	pub fn state_lock(&self) -> parking_lot::MutexGuard<'_, DoubleBuffer<SurfaceState>> {
		self.state.lock()
	}

	#[tracing::instrument(level = "debug", skip_all)]
	pub fn add_commit_handler<F: FnMut(&Surface, &SurfaceState) -> bool + Send + Sync + 'static>(
		&self,
		handler: F,
	) {
		let mut handlers = self.on_commit_handlers.lock();
		handlers.push(Box::new(handler));
	}

	#[tracing::instrument(level = "debug", skip_all)]
	pub fn update_graphics(
		&self,
		dmatexes: &ImportedDmatexs,
		materials: &mut Assets<BevyMaterial>,
		images: &mut Assets<Image>,
	) {
		let Some(buffer) = self.state.lock().current().buffer.clone() else {
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

		if let Some(new_tex) = buffer.buffer.update_tex(dmatexes, images) {
			let material = materials.get_mut(material).unwrap();
			material.base_color_texture.replace(new_tex);
			material.alpha_mode = if buffer.buffer.is_transparent() {
				AlphaMode::Premultiplied
			} else {
				AlphaMode::Opaque
			};
		}

		self.apply_surface_materials();
	}

	#[tracing::instrument(level = "debug", skip_all)]
	pub fn apply_material(&self, model_part: &Arc<ModelPart>) {
		// tracing::info!("uwu applying material");
		self.pending_material_applications.add_raw(model_part)
	}

	#[tracing::instrument(level = "debug", skip_all)]
	fn apply_surface_materials(&self) {
		let Some(mat) = self.material.get() else {
			return;
		};

		for model_node in self.pending_material_applications.get_valid_contents() {
			model_node.replace_material(mat.clone());
		}
		self.pending_material_applications.clear();
	}
	#[tracing::instrument("debug", skip_all)]
	pub fn current_state(&self) -> SurfaceState {
		self.state.lock().current().clone()
	}
	#[tracing::instrument(level = "debug", skip_all)]
	pub fn frame_event(&self) {
		let callbacks = std::mem::take(&mut *self.frame_callbacks.lock());
		if !callbacks.is_empty() {
			let _ = self.message_sink.send(Message::Frame(callbacks));
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

	// pub async fn release_old_buffer(&self, client: &mut Self::Connection) -> Result<()> {
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

	#[tracing::instrument(level = "debug", skip_all)]
	pub fn add_presentation_feedback(&self, feedback: Arc<PresentationFeedback>) {
		self.presentation_feedback.lock().push(feedback);
	}

	pub fn submit_presentation_feedback(
		self: &Arc<Self>,
		display_timestamp: MonotonicTimestamp,
		refresh_cycle: u64,
	) {
		let _ = self.message_sink.send(Message::SendPresentationFeedback {
			surface: self.clone(),
			display_timestamp,
			refresh_cycle,
		});
	}

	#[tracing::instrument(level = "debug", skip_all)]
	pub async fn send_presentation_feedback(
		&self,
		client: &mut Client,
		display_timestamp: MonotonicTimestamp,
		refresh_cycle: u64,
	) -> WaylandResult<()> {
		let feedbacks = self
			.presentation_feedback
			.lock()
			.drain(..)
			.collect::<Vec<_>>();
		for feedback in feedbacks {
			feedback
				.sync_output(
					client,
					feedback.0,
					client.display().output.get().unwrap().id,
				)
				.await?;
			let cycle_lo = refresh_cycle as u32;
			let cycle_hi = (refresh_cycle >> 32) as u32;
			feedback
				.presented(
					client,
					feedback.0,
					display_timestamp.secs_hi(),
					display_timestamp.secs_lo(),
					display_timestamp.subsec_nanos(),
					0,
					cycle_hi,
					cycle_lo,
					Kind::empty(),
				)
				.await?;
		}
		Ok(())
	}
}
impl WlSurface for Surface {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/wayland#wl_surface:request:attach
	#[tracing::instrument(level = "debug", skip_all)]
	async fn attach(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		buffer: Option<ObjectId>,
		_x: i32,
		_y: i32,
	) -> WaylandResult<()> {
		self.state.lock().pending.buffer = buffer.and_then(|b| {
			let buffer = client.get::<Buffer>(b)?;
			let mut usage = Some(BufferUsage::new(client, &buffer));
			Some(BufferState {
				usage: usage.take_if(|_| buffer.uses_buffer_usage()),
				buffer,
			})
		});
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:damage
	#[tracing::instrument(level = "debug", skip_all)]
	async fn damage(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:frame
	#[tracing::instrument(level = "debug", skip_all)]
	async fn frame(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		callback_id: ObjectId,
	) -> WaylandResult<()> {
		let callback = client.insert(callback_id, Callback(callback_id))?;
		self.state.lock().pending.frame_callbacks.push(callback);
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_opaque_region
	#[tracing::instrument(level = "debug", skip_all)]
	async fn set_opaque_region(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_region: Option<ObjectId>,
	) -> WaylandResult<()> {
		// nothing we can really do to repaint behind this so ignore it
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_input_region
	#[tracing::instrument(level = "debug", skip_all)]
	async fn set_input_region(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_region: Option<ObjectId>,
	) -> WaylandResult<()> {
		// too complicated to implement this for now so who the hell cares
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:commit
	#[tracing::instrument(level = "debug", skip_all)]
	async fn commit(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		// we want the upload to complete before we give the image to bevy
		let buffer_option = self
			.state_lock()
			.pending
			.buffer
			.as_ref()
			.map(|b| b.buffer.clone());
		if let Some(buffer) = buffer_option {
			tokio::task::spawn_blocking(move || buffer.on_commit())
				.await
				.unwrap();
		}
		self.state.lock().apply();

		self.state.lock().pending.frame_callbacks.clear();
		let current_state = self.current_state();
		self.frame_callbacks
			.lock()
			.extend(current_state.frame_callbacks.iter().cloned());
		let mut handlers = self.on_commit_handlers.lock();
		handlers.retain_mut(|f| (f)(self, &current_state));
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_buffer_transform
	#[tracing::instrument(level = "debug", skip_all)]
	async fn set_buffer_transform(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_transform: Transform,
	) -> WaylandResult<()> {
		// we just don't have the output transform or fullscreen at all so this optimization is never needed
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:set_buffer_scale
	#[tracing::instrument(level = "debug", skip_all)]
	async fn set_buffer_scale(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		scale: i32,
	) -> WaylandResult<()> {
		self.state.lock().pending.density = scale as f32;
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:damage_buffer
	#[tracing::instrument(level = "debug", skip_all)]
	async fn damage_buffer(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:offset
	#[tracing::instrument(level = "debug", skip_all)]
	async fn offset(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
	) -> WaylandResult<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_surface:request:destroy
	#[tracing::instrument(level = "debug", skip_all)]
	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}
}
impl Drop for Surface {
	fn drop(&mut self) {
		self.role.take();
	}
}
