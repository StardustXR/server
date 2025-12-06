use crate::wayland::{WaylandError, WaylandResult, core::pointer::Pointer};
use mint::Vector2;
use std::sync::Arc;
use waynest::ObjectId;
use waynest_protocols::server::unstable::relative_pointer_unstable_v1::{
	zwp_relative_pointer_manager_v1::*, zwp_relative_pointer_v1::*,
};
use waynest_server::Client as _;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct RelativePointerManager(pub ObjectId);
impl ZwpRelativePointerManagerV1 for RelativePointerManager {
	type Connection = crate::wayland::Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.0);
		Ok(())
	}

	async fn get_relative_pointer(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
		pointer: ObjectId,
	) -> WaylandResult<()> {
		let Some(pointer) = client.get::<Pointer>(pointer) else {
			return Err(WaylandError::MissingObject(pointer));
		};

		let relative_pointer = client.insert(id, RelativePointer(id))?;

		*pointer.relative_pointer.write().await = Arc::downgrade(&relative_pointer);

		Ok(())
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct RelativePointer(pub ObjectId);
impl RelativePointer {
	pub async fn send_relative_motion(
		&self,
		client: &mut crate::wayland::Client,
		delta: Vector2<f32>,
	) -> WaylandResult<()> {
		self.relative_motion(
			client,
			self.0,
			0,
			0,
			(delta.x as f64).into(),
			(delta.y as f64).into(),
			(delta.x as f64).into(),
			(delta.y as f64).into(),
		)
		.await
	}
}
impl ZwpRelativePointerV1 for RelativePointer {
	type Connection = crate::wayland::Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.0);
		Ok(())
	}
}
