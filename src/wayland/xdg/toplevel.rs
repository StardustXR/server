use super::backend::XdgBackend;
use crate::{
	nodes::{
		Node,
		items::panel::{PanelItem, SurfaceId},
	},
	wayland::{
		Client, WaylandResult,
		core::{seat::Seat, surface::Surface},
	},
};
use mint::Vector2;
use parking_lot::Mutex;
use std::sync::Arc;
use waynest::ObjectId;
pub use waynest_protocols::server::stable::xdg_shell::xdg_toplevel::*;

#[derive(Debug)]
pub struct MappedInner {
	pub panel_item_node: Arc<Node>,
	pub panel_item: Arc<PanelItem<XdgBackend>>,
}
impl MappedInner {
	pub fn create(seat: &Arc<Seat>, toplevel: &Arc<Toplevel>, pid: Option<i32>) -> Self {
		let (panel_item_node, panel_item) =
			PanelItem::create(Box::new(XdgBackend::new(seat, toplevel)), pid);

		Self {
			panel_item_node,
			panel_item,
		}
	}
}

#[derive(Debug, Clone)]
struct ToplevelData {
	parent: Option<u64>,
	app_id: Option<String>,
	title: Option<String>,
	activated: bool,
	fullscreen: bool,
	pub size: Option<Vector2<u32>>,
}
impl Default for ToplevelData {
	fn default() -> Self {
		Self {
			parent: None,
			app_id: None,
			title: None,
			activated: true,
			fullscreen: false,
			size: None,
		}
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct Toplevel {
	pub id: ObjectId,
	xdg_surface: Arc<super::surface::Surface>,
	pub mapped: Mutex<Option<MappedInner>>,
	data: Mutex<ToplevelData>,
}
impl Toplevel {
	pub fn new(
		object_id: ObjectId,
		wl_surface: Arc<Surface>,
		xdg_surface: Arc<super::surface::Surface>,
	) -> Self {
		let _ = wl_surface.surface_id.set(SurfaceId::Toplevel(()));

		Toplevel {
			id: object_id,
			xdg_surface,
			mapped: Mutex::new(None),
			data: Mutex::new(ToplevelData::default()),
		}
	}

	pub fn wl_surface(&self) -> &Arc<Surface> {
		&self.xdg_surface.wl_surface
	}

	pub fn title(&self) -> Option<String> {
		self.data.lock().title.clone()
	}
	pub fn app_id(&self) -> Option<String> {
		self.data.lock().app_id.clone()
	}
	pub fn parent(&self) -> Option<u64> {
		self.data.lock().parent
	}

	pub fn set_size(&self, size: Option<Vector2<u32>>) {
		self.data.lock().size = size;
	}

	pub fn set_activated(&self, activated: bool) {
		self.data.lock().activated = activated;
	}

	// Helper to clamp size against constraints
	fn clamp_size(&self, size: Vector2<u32>) -> Vector2<u32> {
		let state = self.wl_surface().current_state();
		let mut clamped = size;

		if let Some(min_size) = state.min_size {
			clamped.x = clamped.x.max(min_size.x);
			clamped.y = clamped.y.max(min_size.y);
		}
		if let Some(max_size) = state.max_size {
			clamped.x = clamped.x.min(max_size.x);
			clamped.y = clamped.y.min(max_size.y);
		}
		clamped
	}

	pub async fn reconfigure(&self, client: &mut Client) -> WaylandResult<()> {
		let data = self.data.lock().clone();

		// Use the explicitly set size, applying constraints
		let size = data.size.map(|s| self.clamp_size(s));

		let mut states = vec![
			State::TiledTop,
			State::TiledLeft,
			State::TiledRight,
			State::TiledBottom,
			if data.fullscreen {
				State::Fullscreen
			} else {
				State::Maximized
			},
		];
		if data.activated {
			states.push(State::Activated);
		}

		self.configure(
			client,
			self.id,
			size.map(|v| v.x as i32).unwrap_or(0),
			size.map(|v| v.y as i32).unwrap_or(0),
			states
				.into_iter()
				.flat_map(|x| (x as u32).to_ne_bytes())
				.collect(),
		)
		.await?;
		self.xdg_surface.reconfigure(client).await?;
		Ok(())
	}
}
impl XdgToplevel for Toplevel {
	type Connection = crate::wayland::Client;

	async fn set_parent(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		parent: Option<ObjectId>,
	) -> WaylandResult<()> {
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
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		title: String,
	) -> WaylandResult<()> {
		self.data.lock().title.replace(title);
		Ok(())
	}

	async fn set_app_id(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		app_id: String,
	) -> WaylandResult<()> {
		self.data.lock().app_id.replace(app_id);
		Ok(())
	}

	async fn show_window_menu(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
		_x: i32,
		_y: i32,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn r#move(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn resize(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
		_edges: ResizeEdge,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_max_size(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		width: i32,
		height: i32,
	) -> WaylandResult<()> {
		self.wl_surface().state_lock().pending.max_size = if width == 0 && height == 0 {
			None
		} else {
			Some([width as u32, height as u32].into())
		};
		Ok(())
	}

	async fn set_min_size(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		width: i32,
		height: i32,
	) -> WaylandResult<()> {
		self.xdg_surface.wl_surface.state_lock().pending.min_size = if width == 0 && height == 0 {
			None
		} else {
			Some([width as u32, height as u32].into())
		};
		Ok(())
	}

	async fn set_maximized(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn unset_maximized(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_fullscreen(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_output: Option<ObjectId>,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn unset_fullscreen(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_minimized(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		self.mapped.lock().take();
		Ok(())
	}
}
impl Drop for Toplevel {
	fn drop(&mut self) {
		self.mapped.lock().take();
	}
}
