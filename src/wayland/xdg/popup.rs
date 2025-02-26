use waynest::server::protocol::stable::xdg_shell::xdg_positioner::*;
use waynest::server::{Client, Dispatcher, Result};
use waynest::wire::ObjectId;

#[derive(Debug, Dispatcher, Default)]
pub struct Positioner {
	// TOOD: impl this
}
impl XdgPositioner for Positioner {
	async fn set_size(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_anchor_rect(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_anchor(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_anchor: Anchor,
	) -> Result<()> {
		Ok(())
	}

	async fn set_gravity(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_gravity: Gravity,
	) -> Result<()> {
		Ok(())
	}

	async fn set_constraint_adjustment(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_constraint_adjustment: ConstraintAdjustment,
	) -> Result<()> {
		Ok(())
	}

	async fn set_offset(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_reactive(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn set_parent_size(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_parent_width: i32,
		_parent_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_parent_configure(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_serial: u32,
	) -> Result<()> {
		Ok(())
	}

	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}
