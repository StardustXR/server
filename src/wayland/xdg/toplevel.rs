use super::backend::XdgBackend;
use crate::nodes::{
	items::panel::{Geometry, PanelItem, ToplevelInfo},
	Node,
};
use mint::Vector2;
use parking_lot::Mutex;
use std::sync::Arc;
pub use waynest::server::protocol::stable::xdg_shell::xdg_toplevel::*;
use waynest::{
	server::{Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Dispatcher)]
pub struct Toplevel {
	panel_item_node: Arc<Node>,
	panel_item: Arc<PanelItem<XdgBackend>>,
	pub info: Mutex<ToplevelInfo>,
}
impl Toplevel {
	pub fn new(pid: Option<i32>, size: Vector2<u32>) -> Self {
		let (panel_item_node, panel_item) = PanelItem::create(Box::new(XdgBackend::default()), pid);

		Toplevel {
			panel_item_node,
			panel_item,
			info: Mutex::new(ToplevelInfo {
				parent: None,
				title: None,
				app_id: None,
				size,
				min_size: None,
				max_size: None,
				logical_rectangle: Geometry {
					origin: [0; 2].into(),
					size,
				},
			}),
		}
	}
}
impl XdgToplevel for Toplevel {
	async fn set_parent(
		&self,
		_object: &Object,
		client: &mut Client,
		parent: Option<ObjectId>,
	) -> Result<()> {
		if let Some(parent) = parent {
			if let Some(parent_object) = client.get_object(&parent) {
				let parent_toplevel = parent_object.as_dispatcher::<Toplevel>()?;
				self.info
					.lock()
					.parent
					.replace(parent_toplevel.panel_item_node.get_id());
			}
		} else {
			self.info.lock().parent.take();
		}

		Ok(())
	}

	async fn set_title(
		&self,
		_object: &Object,
		_client: &mut Client,
		_title: String,
	) -> Result<()> {
		// FIXME: change  state

		Ok(())
	}

	async fn set_app_id(
		&self,
		_object: &Object,
		_client: &mut Client,
		_app_id: String,
	) -> Result<()> {
		// FIXME: change  state

		Ok(())
	}

	async fn show_window_menu(
		&self,
		_object: &Object,
		_client: &mut Client,
		_seat: ObjectId,
		_serial: u32,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		todo!()
	}

	async fn r#move(
		&self,
		_object: &Object,
		_client: &mut Client,
		_seat: ObjectId,
		_serial: u32,
	) -> Result<()> {
		todo!()
	}

	async fn resize(
		&self,
		_object: &Object,
		_client: &mut Client,
		_seat: ObjectId,
		_serial: u32,
		_edges: ResizeEdge,
	) -> Result<()> {
		todo!()
	}

	async fn set_max_size(
		&self,
		_object: &Object,
		_client: &mut Client,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		todo!()
	}

	async fn set_min_size(
		&self,
		_object: &Object,
		_client: &mut Client,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		todo!()
	}

	async fn set_maximized(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		todo!()
	}

	async fn unset_maximized(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		todo!()
	}

	async fn set_fullscreen(
		&self,
		_object: &Object,
		_client: &mut Client,
		_output: Option<ObjectId>,
	) -> Result<()> {
		todo!()
	}

	async fn unset_fullscreen(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		todo!()
	}

	async fn set_minimized(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		todo!()
	}

	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}
