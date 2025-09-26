use crate::{nodes::items::panel::Geometry, wayland::WaylandResult};
use mint::Vector2;
use parking_lot::Mutex;
use waynest::ObjectId;
use waynest_protocols::server::stable::xdg_shell::xdg_positioner::*;

#[derive(Debug, Clone, Copy)]
pub struct PositionerData {
	pub size: Vector2<u32>,
	pub anchor_rect: Geometry,
	pub offset: Vector2<i32>,
	pub anchor: Anchor,
	pub gravity: Gravity,
	pub constraint_adjustment: ConstraintAdjustment,
	pub reactive: bool,
	pub parent_size: Vector2<u32>,
}
impl PositionerData {
	fn gravity_has_edge(&self, edge: Gravity) -> bool {
		match edge {
			Gravity::Top => {
				self.gravity == Gravity::Top
					|| self.gravity == Gravity::TopLeft
					|| self.gravity == Gravity::TopRight
			}
			Gravity::Bottom => {
				self.gravity == Gravity::Bottom
					|| self.gravity == Gravity::BottomLeft
					|| self.gravity == Gravity::BottomRight
			}
			Gravity::Left => {
				self.gravity == Gravity::Left
					|| self.gravity == Gravity::TopLeft
					|| self.gravity == Gravity::BottomLeft
			}
			Gravity::Right => {
				self.gravity == Gravity::Right
					|| self.gravity == Gravity::TopRight
					|| self.gravity == Gravity::BottomRight
			}
			_ => unreachable!(),
		}
	}
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

		let mut geometry = Geometry {
			origin: [
				anchor_point.x + self.offset.x,
				anchor_point.y + self.offset.y,
			]
			.into(),
			size: self.size,
		};

		// apply gravity
		if self.gravity_has_edge(Gravity::Top) {
			geometry.origin.y -= geometry.size.y as i32;
		} else if !self.gravity_has_edge(Gravity::Bottom) {
			geometry.origin.y -= (geometry.size.y / 2) as i32;
		}

		if self.gravity_has_edge(Gravity::Left) {
			geometry.origin.x -= geometry.size.x as i32;
		} else if !self.gravity_has_edge(Gravity::Right) {
			geometry.origin.x -= (geometry.size.x / 2) as i32;
		}

		geometry
	}
}
impl Default for PositionerData {
	fn default() -> Self {
		Self {
			size: [0; 2].into(),
			anchor_rect: Default::default(),
			offset: [0, 0].into(),
			anchor: Anchor::TopLeft,
			gravity: Gravity::TopLeft,
			constraint_adjustment: ConstraintAdjustment::empty(),
			reactive: false,
			parent_size: [0; 2].into(),
		}
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
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
	type Connection = crate::wayland::Client;

	async fn set_size(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		let mut data = self.data.lock();
		data.size = [_width.max(0) as u32, _height.max(0) as u32].into();
		data.reactive = true;
		Ok(())
	}

	async fn set_anchor_rect(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		let mut data = self.data.lock();
		data.anchor_rect.origin = [_x, _y].into();
		data.anchor_rect.size = [_width.max(0) as u32, _height.max(0) as u32].into();
		data.offset = [0, 0].into();
		Ok(())
	}

	async fn set_anchor(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_anchor: Anchor,
	) -> WaylandResult<()> {
		let mut data = self.data.lock();
		data.anchor = _anchor;
		Ok(())
	}

	async fn set_gravity(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		gravity: Gravity,
	) -> WaylandResult<()> {
		let mut data = self.data.lock();
		data.gravity = gravity;
		Ok(())
	}

	async fn set_constraint_adjustment(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_constraint_adjustment: ConstraintAdjustment,
	) -> WaylandResult<()> {
		let mut data = self.data.lock();
		data.constraint_adjustment = _constraint_adjustment;
		Ok(())
	}

	async fn set_offset(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
	) -> WaylandResult<()> {
		let mut data = self.data.lock();
		data.offset.x += _x;
		data.offset.y += _y;
		Ok(())
	}

	async fn set_reactive(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_parent_size(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_parent_width: i32,
		_parent_height: i32,
	) -> WaylandResult<()> {
		let mut data = self.data.lock();
		data.parent_size.x = _parent_width.max(0) as u32;
		data.parent_size.y = _parent_height.max(0) as u32;
		Ok(())
	}

	async fn set_parent_configure(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_serial: u32,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn destroy(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}
}
