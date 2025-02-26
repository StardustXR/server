use super::backend::XdgBackend;
use crate::{
	nodes::{Node, items::panel::PanelItem},
	wayland::core::surface::Surface,
};
use parking_lot::Mutex;
use std::sync::Arc;
pub use waynest::server::protocol::stable::xdg_shell::xdg_toplevel::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug)]
pub struct Mapped {
	pub panel_item_node: Arc<Node>,
	pub panel_item: Arc<PanelItem<XdgBackend>>,
}
impl Mapped {
	pub fn create(toplevel: Arc<Toplevel>, pid: Option<i32>) -> Self {
		let (panel_item_node, panel_item) =
			PanelItem::create(Box::new(XdgBackend { toplevel }), pid);

		Self {
			panel_item_node,
			panel_item,
		}
	}
}
#[derive(Debug, Default)]
struct ToplevelData {
	parent: Option<u64>,
	app_id: Option<String>,
	title: Option<String>,
}

#[derive(Debug, Dispatcher)]
pub struct Toplevel {
	pub object_id: ObjectId,
	pub wl_surface: Arc<Surface>,
	pub mapped: Mutex<Option<Mapped>>,
	data: Mutex<ToplevelData>,
}
impl Toplevel {
	pub fn new(object_id: ObjectId, wl_surface: Arc<Surface>) -> Self {
		Toplevel {
			object_id,
			wl_surface,
			mapped: Mutex::new(None),
			data: Mutex::new(ToplevelData::default()),
		}
	}

	pub fn parent(&self) -> Option<u64> {
		self.data.lock().parent
	}
	pub fn app_id(&self) -> Option<String> {
		self.data.lock().app_id.clone()
	}
	pub fn title(&self) -> Option<String> {
		self.data.lock().title.clone()
	}
}
impl XdgToplevel for Toplevel {
	async fn set_parent(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		parent: Option<ObjectId>,
	) -> Result<()> {
		// Handle case where parent is specified
		if let Some(parent) = parent {
			// Per spec: parent must be another xdg_toplevel surface
			if let Some(parent_toplevel) = client.get::<Toplevel>(parent) {
				let Some(mapped) = &*parent_toplevel.mapped.lock() else {
					// Per spec: parent surfaces must be mapped before being used as a parent
					// Setting an unmapped window as parent should raise a protocol error
					// For now we just unset the parent as a fallback
					self.data.lock().parent.take();
					return Ok(());
				};

				// Per spec: store parent to ensure this surface is stacked above parent
				// and other ancestor surfaces. Used for proper window stacking order.
				self.data
					.lock()
					.parent
					.replace(mapped.panel_item_node.get_id());
			}
		} else {
			// Per spec: null parent unsets the parent, making this a top-level window
			// This allows converting child windows back to independent top-level windows
			self.data.lock().parent.take();
		}

		Ok(())
	}

	async fn set_title(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		title: String,
	) -> Result<()> {
		self.data.lock().title.replace(title);
		Ok(())
	}

	async fn set_app_id(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		app_id: String,
	) -> Result<()> {
		self.data.lock().app_id.replace(app_id);
		Ok(())
	}

	async fn show_window_menu(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn r#move(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
	) -> Result<()> {
		Ok(())
	}

	async fn resize(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
		_edges: ResizeEdge,
	) -> Result<()> {
		Ok(())
	}

	async fn set_max_size(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_min_size(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_maximized(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn unset_maximized(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn set_fullscreen(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_output: Option<ObjectId>,
	) -> Result<()> {
		Ok(())
	}

	async fn unset_fullscreen(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn set_minimized(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}
