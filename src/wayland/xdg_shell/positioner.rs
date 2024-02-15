use crate::{nodes::items::panel::Geometry, wayland::state::WaylandState};
use mint::Vector2;
use parking_lot::Mutex;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::xdg_positioner::{
		self, Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
	},
	wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource},
};
use tracing::{debug, warn};
use wayland_backend::protocol::WEnum;

#[derive(Debug, Clone, Copy)]
pub struct PositionerData {
	size: Vector2<u32>,
	anchor_rect_pos: Vector2<i32>,
	anchor_rect_size: Vector2<u32>,
	anchor: Anchor,
	gravity: Gravity,
	constraint_adjustment: ConstraintAdjustment,
	offset: Vector2<i32>,
	reactive: bool,
}
impl Default for PositionerData {
	fn default() -> Self {
		Self {
			size: Vector2::from([0; 2]),
			anchor_rect_pos: Vector2::from([0; 2]),
			anchor_rect_size: Vector2::from([0; 2]),
			anchor: Anchor::None,
			gravity: Gravity::None,
			constraint_adjustment: ConstraintAdjustment::None,
			offset: Vector2::from([0; 2]),
			reactive: false,
		}
	}
}

impl PositionerData {
	fn anchor_has_edge(&self, edge: Anchor) -> bool {
		match edge {
			Anchor::Top => {
				self.anchor == Anchor::Top
					|| self.anchor == Anchor::TopLeft
					|| self.anchor == Anchor::TopRight
			}
			Anchor::Bottom => {
				self.anchor == Anchor::Bottom
					|| self.anchor == Anchor::BottomLeft
					|| self.anchor == Anchor::BottomRight
			}
			Anchor::Left => {
				self.anchor == Anchor::Left
					|| self.anchor == Anchor::TopLeft
					|| self.anchor == Anchor::BottomLeft
			}
			Anchor::Right => {
				self.anchor == Anchor::Right
					|| self.anchor == Anchor::TopRight
					|| self.anchor == Anchor::BottomRight
			}
			_ => unreachable!(),
		}
	}

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

	pub fn get_pos(&self) -> Vector2<i32> {
		let mut pos = self.offset;

		if self.anchor_has_edge(Anchor::Top) {
			pos.y += self.anchor_rect_pos.y;
		} else if self.anchor_has_edge(Anchor::Bottom) {
			pos.y += self.anchor_rect_pos.y + self.anchor_rect_size.y as i32;
		} else {
			pos.y += self.anchor_rect_pos.y + self.anchor_rect_size.y as i32 / 2;
		}

		if self.anchor_has_edge(Anchor::Left) {
			pos.x += self.anchor_rect_pos.x;
		} else if self.anchor_has_edge(Anchor::Right) {
			pos.x += self.anchor_rect_pos.x + self.anchor_rect_size.x as i32;
		} else {
			pos.x += self.anchor_rect_pos.x + self.anchor_rect_size.x as i32 / 2;
		}

		if self.gravity_has_edge(Gravity::Top) {
			pos.y -= self.size.y as i32;
		} else if !self.gravity_has_edge(Gravity::Bottom) {
			pos.y -= self.size.y as i32 / 2;
		}

		if self.gravity_has_edge(Gravity::Left) {
			pos.x -= self.size.x as i32;
		} else if !self.gravity_has_edge(Gravity::Right) {
			pos.x -= self.size.x as i32 / 2;
		}

		pos
	}
}
impl From<PositionerData> for Geometry {
	fn from(value: PositionerData) -> Self {
		Geometry {
			origin: value.get_pos(),
			size: value.size,
		}
	}
}

impl Dispatch<XdgPositioner, Mutex<PositionerData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		positioner: &XdgPositioner,
		request: xdg_positioner::Request,
		data: &Mutex<PositionerData>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_positioner::Request::SetSize { width, height } => {
				debug!(?positioner, width, height, "Set positioner size");
				data.lock().size = Vector2::from([width as u32, height as u32]);
			}
			xdg_positioner::Request::SetAnchorRect {
				x,
				y,
				width,
				height,
			} => {
				if width < 1 || height < 1 {
					positioner.post_error(
						xdg_positioner::Error::InvalidInput,
						"Invalid size for positioner's anchor rectangle.",
					);
					warn!(
						?positioner,
						width, height, "Invalid size for positioner's anchor rectangle"
					);
					return;
				}

				debug!(
					?positioner,
					x, y, width, height, "Set positioner anchor rectangle"
				);
				let mut data = data.lock();
				data.anchor_rect_pos = [x, y].into();
				data.anchor_rect_size = [width as u32, height as u32].into();
			}
			xdg_positioner::Request::SetAnchor { anchor } => {
				if let WEnum::Value(anchor) = anchor {
					debug!(?positioner, ?anchor, "Set positioner anchor");
					data.lock().anchor = anchor;
				}
			}
			xdg_positioner::Request::SetGravity { gravity } => {
				if let WEnum::Value(gravity) = gravity {
					debug!(?positioner, ?gravity, "Set positioner gravity");
					data.lock().gravity = gravity;
				}
			}
			xdg_positioner::Request::SetConstraintAdjustment {
				constraint_adjustment,
			} => {
				debug!(
					?positioner,
					constraint_adjustment, "Set positioner constraint adjustment"
				);
				let Some(constraint_adjustment) =
					ConstraintAdjustment::from_bits(constraint_adjustment)
				else {
					return;
				};
				data.lock().constraint_adjustment = constraint_adjustment;
			}
			xdg_positioner::Request::SetOffset { x, y } => {
				debug!(?positioner, x, y, "Set positioner offset");
				data.lock().offset = [x, y].into();
			}
			xdg_positioner::Request::SetReactive => {
				debug!(?positioner, "Set positioner reactive");
				data.lock().reactive = true;
			}
			xdg_positioner::Request::SetParentSize {
				parent_width,
				parent_height,
			} => {
				debug!(
					?positioner,
					parent_width, parent_height, "Set positioner parent size"
				);
			}
			xdg_positioner::Request::SetParentConfigure { serial } => {
				debug!(?positioner, serial, "Set positioner parent size");
			}
			xdg_positioner::Request::Destroy => (),
			_ => unreachable!(),
		}
	}
}
