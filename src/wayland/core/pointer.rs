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
	focused_surface: Mutex<Weak<Surface>>,
}
impl Pointer {
	pub fn new(id: ObjectId) -> Self {
		Self {
			id,
			focused_surface: Mutex::new(Weak::new()),
		}
	}

	pub async fn handle_pointer_motion(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		position: Vector2<f32>,
	) -> Result<()> {
		tracing::info!(
			"Handling pointer motion at ({}, {})",
			position.x,
			position.y
		);
		let mut focused = self.focused_surface.lock().await;

		// If we're entering a new surface
		if focused.as_ptr() != Arc::as_ptr(&surface) {
			tracing::info!("Surface transition detected");
			// Send leave to old surface if it exists and is still alive
			if let Some(old_surface) = focused.upgrade() {
				let serial = client.next_event_serial();
				tracing::info!("Sending leave event with serial {}", serial);
				self.leave(client, self.id, serial, old_surface.id).await?;
			}

			// Send enter to new surface
			let serial = client.next_event_serial();
			tracing::info!(
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
		tracing::info!("Sending motion event to surface");
		self.motion(
			client,
			self.id,
			0, // time
			fixed_from_f32(position.x),
			fixed_from_f32(position.y),
		)
		.await?;

		Ok(())
	}

	pub async fn handle_pointer_button(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		button: u32,
		pressed: bool,
	) -> Result<()> {
		tracing::info!(
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
		.await
	}
	pub async fn handle_pointer_scroll(
		&self,
		client: &mut Client,
		_surface: Arc<Surface>,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) -> Result<()> {
		tracing::info!(
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
		if let Some(steps) = scroll_steps {
			self.axis_discrete(client, self.id, Axis::HorizontalScroll, steps.x as i32)
				.await?;
			self.axis_discrete(client, self.id, Axis::VerticalScroll, steps.y as i32)
				.await?;
		}
		Ok(())
	}

	pub async fn reset(&self, client: &mut Client) -> Result<()> {
		let mut focused = self.focused_surface.lock().await;
		if let Some(old_surface) = focused.upgrade() {
			let serial = client.next_event_serial();
			self.leave(client, self.id, serial, old_surface.id).await?;
		}
		*focused = Weak::new();
		Ok(())
	}
}

impl WlPointer for Pointer {
	async fn set_cursor(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_serial: u32,
		_surface: Option<ObjectId>,
		_hotspot_x: i32,
		_hotspot_y: i32,
	) -> Result<()> {
		Ok(())
	}
	async fn release(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}
