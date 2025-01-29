use waynest::server::protocol::stable::xdg_shell::xdg_positioner::*;
use waynest::server::{Client, Dispatcher, Object, Result};

#[derive(Debug, Dispatcher, Default)]
pub struct Positioner {
	// TOOD: impl this
}
impl XdgPositioner for Positioner {
	async fn set_size(
		&self,
		_object: &Object,
		_client: &mut Client,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_anchor_rect(
		&self,
		_object: &Object,
		_client: &mut Client,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_anchor(
		&self,
		_object: &Object,
		_client: &mut Client,
		_anchor: Anchor,
	) -> Result<()> {
		Ok(())
	}

	async fn set_gravity(
		&self,
		_object: &Object,
		_client: &mut Client,
		_gravity: Gravity,
	) -> Result<()> {
		Ok(())
	}

	async fn set_constraint_adjustment(
		&self,
		_object: &Object,
		_client: &mut Client,
		_constraint_adjustment: ConstraintAdjustment,
	) -> Result<()> {
		Ok(())
	}

	async fn set_offset(
		&self,
		_object: &Object,
		_client: &mut Client,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_reactive(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}

	async fn set_parent_size(
		&self,
		_object: &Object,
		_client: &mut Client,
		_parent_width: i32,
		_parent_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_parent_configure(
		&self,
		_object: &Object,
		_client: &mut Client,
		_serial: u32,
	) -> Result<()> {
		Ok(())
	}

	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}
