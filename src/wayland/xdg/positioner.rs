use crate::nodes::items::panel::Geometry;
use mint::Vector2;
use parking_lot::Mutex;
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::stable::xdg_shell::xdg_positioner::{
			Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
		},
	},
	wire::ObjectId,
};

#[derive(Debug, Clone, Copy)]
pub struct PositionerData {
	pub size: Vector2<u32>,
	pub anchor_rect: Geometry,
	pub offset: Vector2<i32>,
	pub anchor: Anchor,
	pub constraint_adjustment: ConstraintAdjustment,
	pub reactive: bool,
	pub parent_size: Vector2<u32>,
}
impl PositionerData {
	pub fn infinite_geometry(&self) -> Geometry {
		let anchor_point = match self.anchor {
			Anchor::TopLeft => self.anchor_rect.origin,
			Anchor::Top => [
				self.anchor_rect.origin.x + (self.anchor_rect.size.x / 2) as i32,
				self.anchor_rect.origin.y,
			]
			.into(),
			Anchor::TopRight => [
				self.anchor_rect.origin.x + self.anchor_rect.size.x as i32,
				self.anchor_rect.origin.y,
			]
			.into(),
			Anchor::Left => [
				self.anchor_rect.origin.x,
				self.anchor_rect.origin.y + (self.anchor_rect.size.y / 2) as i32,
			]
			.into(),
			Anchor::Right => [
				self.anchor_rect.origin.x + self.anchor_rect.size.x as i32,
				self.anchor_rect.origin.y + (self.anchor_rect.size.y / 2) as i32,
			]
			.into(),
			Anchor::BottomLeft => [
				self.anchor_rect.origin.x,
				self.anchor_rect.origin.y + self.anchor_rect.size.y as i32,
			]
			.into(),
			Anchor::Bottom => [
				self.anchor_rect.origin.x + (self.anchor_rect.size.x / 2) as i32,
				self.anchor_rect.origin.y + self.anchor_rect.size.y as i32,
			]
			.into(),
			Anchor::BottomRight => [
				self.anchor_rect.origin.x + self.anchor_rect.size.x as i32,
				self.anchor_rect.origin.y + self.anchor_rect.size.y as i32,
			]
			.into(),
			_ => [
				self.anchor_rect.origin.x + (self.anchor_rect.size.x / 2) as i32,
				self.anchor_rect.origin.y + (self.anchor_rect.size.y / 2) as i32,
			]
			.into(),
		};

		let mut position = anchor_point;

		// Apply gravity
		if self
			.constraint_adjustment
			.contains(ConstraintAdjustment::FlipX)
		{
			position.x -= self.size.x as i32;
		}
		if self
			.constraint_adjustment
			.contains(ConstraintAdjustment::FlipY)
		{
			position.y -= self.size.y as i32;
		}

		// Apply offset
		position.x += self.offset.x;
		position.y += self.offset.y;

		Geometry {
			origin: position,
			size: self.size,
		}
	}
}
impl Default for PositionerData {
	fn default() -> Self {
		Self {
			size: [0; 2].into(),
			anchor_rect: Default::default(),
			offset: [0, 0].into(),
			anchor: Anchor::TopLeft,
			constraint_adjustment: ConstraintAdjustment::empty(),
			reactive: false,
			parent_size: [0; 2].into(),
		}
	}
}

#[derive(Debug, Dispatcher)]
pub struct Positioner {
	data: Mutex<PositionerData>,
}
impl Default for Positioner {
	fn default() -> Self {
		Self {
			data: Mutex::new(PositionerData::default()),
		}
	}
}
impl Positioner {
	pub fn data(&self) -> PositionerData {
		*self.data.lock()
	}
}
impl XdgPositioner for Positioner {
	async fn set_size(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		let mut data = self.data.lock();
		data.size = [_width.max(0) as u32, _height.max(0) as u32].into();
		data.reactive = true;
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
		let mut data = self.data.lock();
		data.anchor_rect.origin = [_x, _y].into();
		data.anchor_rect.size = [_width.max(0) as u32, _height.max(0) as u32].into();
		data.offset = [0, 0].into();
		Ok(())
	}

	async fn set_anchor(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_anchor: Anchor,
	) -> Result<()> {
		let mut data = self.data.lock();
		data.anchor = _anchor;
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
		let mut data = self.data.lock();
		data.constraint_adjustment = _constraint_adjustment;
		Ok(())
	}

	async fn set_offset(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		let mut data = self.data.lock();
		data.offset.x += _x;
		data.offset.y += _y;
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
		let mut data = self.data.lock();
		data.parent_size.x = _parent_width.max(0) as u32;
		data.parent_size.y = _parent_height.max(0) as u32;
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
