use crate::nodes::items::panel::Geometry;
use crate::wayland::core::{seat::fixed_from_f32, surface::Surface};
use mint::Vector2;
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::Mutex;
use tracing;
pub use waynest::server::protocol::core::wayland::wl_pointer::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Dispatcher)]
pub struct Pointer {
	pub id: ObjectId,
	version: u32,
	focused_surface: Mutex<Weak<Surface>>,
	cursor_surface: Mutex<Option<Arc<Surface>>>,
}
impl Pointer {
	pub fn new(id: ObjectId, version: u32) -> Self {
		Self {
			id,
			version,
			focused_surface: Mutex::new(Weak::new()),
			cursor_surface: Mutex::new(None),
		}
	}

	pub async fn handle_pointer_motion(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		position: Vector2<f32>,
	) -> Result<()> {
		tracing::debug!(
			"Handling pointer motion at ({}, {})",
			position.x,
			position.y
		);
		let mut focused = self.focused_surface.lock().await;

		// If we're entering a new surface
		if focused.as_ptr() != Arc::as_ptr(&surface) {
			tracing::debug!("Surface transition detected");
			// Send leave to old surface if it exists and is still alive
			if let Some(old_surface) = focused.upgrade() {
				let serial = client.next_event_serial();
				tracing::debug!("Sending leave event with serial {}", serial);
				self.leave(client, self.id, serial, old_surface.id).await?;
			}

			// Send enter to new surface
			let serial = client.next_event_serial();
			tracing::debug!(
				"Sending enter event with serial {} to surface {:?}",
				serial,
				surface.id
			);
			self.enter(
				client,
				self.id,
				serial,
				surface.id,
				fixed_from_f32(position.x),
				fixed_from_f32(position.y),
			)
			.await?;

			// Update focused surface
			*focused = Arc::downgrade(&surface);
		}

		// Send motion event to current surface
		tracing::debug!("Sending motion event to surface");
		self.motion(
			client,
			self.id,
			0, // time
			fixed_from_f32(position.x),
			fixed_from_f32(position.y),
		)
		.await?;
		if self.version >= 5 {
			self.frame(client, self.id).await?;
		}

		Ok(())
	}

	pub async fn handle_pointer_button(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		button: u32,
		pressed: bool,
	) -> Result<()> {
		tracing::debug!(
			"Handling pointer button {} {} on surface {:?}",
			button,
			if pressed { "pressed" } else { "released" },
			surface.id
		);
		let serial = client.next_event_serial();
		self.button(
			client,
			self.id,
			serial,
			0, // time
			button,
			if pressed {
				ButtonState::Pressed
			} else {
				ButtonState::Released
			},
		)
		.await?;
		self.frame(client, self.id).await
	}
	pub async fn handle_pointer_scroll(
		&self,
		client: &mut Client,
		_surface: Arc<Surface>,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) -> Result<()> {
		tracing::debug!(
			"Handling pointer scroll: distance={:?}, steps={:?}",
			scroll_distance,
			scroll_steps
		);
		if let Some(distance) = scroll_distance {
			self.axis(
				client,
				self.id,
				0, // time
				Axis::HorizontalScroll,
				fixed_from_f32(distance.x),
			)
			.await?;
			self.axis(
				client,
				self.id,
				0, // time
				Axis::VerticalScroll,
				fixed_from_f32(distance.y),
			)
			.await?;
		}
		if self.version >= 5 {
			if let Some(steps) = scroll_steps {
				self.axis_discrete(client, self.id, Axis::HorizontalScroll, steps.x as i32)
					.await?;
				self.axis_discrete(client, self.id, Axis::VerticalScroll, steps.y as i32)
					.await?;
			}
			self.frame(client, self.id).await?;
		}
		Ok(())
	}

	pub async fn reset(&self, client: &mut Client) -> Result<()> {
		let mut focused = self.focused_surface.lock().await;
		if let Some(old_surface) = focused.upgrade() {
			let serial = client.next_event_serial();
			self.leave(client, self.id, serial, old_surface.id).await?;
			self.frame(client, self.id).await?;
		}
		*focused = Weak::new();
		Ok(())
	}

	pub async fn cursor_surface(&self) -> Option<Arc<Surface>> {
		self.cursor_surface.lock().await.clone()
	}
}

impl WlPointer for Pointer {
	/// https://wayland.app/protocols/wayland#wl_pointer:request:set_cursor
	async fn set_cursor(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		_serial: u32,
		surface: Option<ObjectId>,
		hotspot_x: i32,
		hotspot_y: i32,
	) -> Result<()> {
		if let Some(focused_surface) = self.focused_surface.lock().await.upgrade()
			&& let Some(panel_item) = focused_surface.panel_item.lock().upgrade()
		{
			panel_item.set_cursor(surface.and_then(|s| client.get::<Surface>(s)).map(|s| {
				let size = s
					.current_state()
					.buffer
					.map(|b| b.buffer.size())
					.unwrap_or([16; 2].into());
				Geometry {
					origin: [-hotspot_x, -hotspot_y].into(),
					size: [size.x as u32, size.y as u32].into(),
				}
			}));
		}
		let Some(surface) = surface else {
			return Ok(());
		};
		let Some(surface) = client.get::<Surface>(surface) else {
			return Ok(());
		};
		self.cursor_surface.lock().await.replace(surface);
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_pointer:request:release
	async fn release(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}
