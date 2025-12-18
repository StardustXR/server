use super::surface::{Surface, SurfaceRole};
use crate::nodes::items::panel::{ChildInfo, Geometry, SurfaceId};
use crate::wayland::util::{BufferedState, SurfaceCommitAwareBuffer};
use crate::wayland::{WaylandError, WaylandResult};
use mint::Vector2;
use parking_lot::Mutex;
use rand::Rng;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use waynest::ObjectId;
use waynest_protocols::server::core::wayland::wl_subcompositor::{self, WlSubcompositor};
use waynest_protocols::server::core::wayland::wl_subsurface::WlSubsurface;
use waynest_server::{Client as _, RequestDispatcher};

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = WaylandError, connection = crate::wayland::Client)]
pub struct Subcompositor;

impl WlSubcompositor for Subcompositor {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/wayland#wl_subcompositor:request:destroy
	async fn destroy(
		&self,
		client: &mut Self::Connection,
		sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(sender_id);
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_subcompositor:request:get_subsurface
	async fn get_subsurface(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
		surface_id: ObjectId,
		parent_id: ObjectId,
	) -> WaylandResult<()> {
		let Some(parent) = client.get::<Surface>(parent_id) else {
			return Err(WaylandError::Fatal {
				object_id: parent_id,
				code: wl_subcompositor::Error::BadSurface as u32,
				message: "Parent surface does not exist",
			});
		};

		let Some(surface) = client.get::<Surface>(surface_id) else {
			return Err(WaylandError::Fatal {
				object_id: surface_id,
				code: wl_subcompositor::Error::BadSurface as u32,
				message: "Surface does not exist",
			});
		};

		// Set the subsurface role
		surface
			.try_set_role(SurfaceRole::Subsurface, wl_subcompositor::Error::BadSurface)
			.await?;

		// Create the subsurface
		let subsurface = Arc::new(Subsurface::new(id, surface.clone(), parent.clone()));
		client.insert_raw(id, subsurface.clone())?;

		// Set up commit handler and register child
		subsurface.setup();

		Ok(())
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SubsurfaceState {
	position: (i32, i32),
	z_order: i32,
}

impl Default for SubsurfaceState {
	fn default() -> Self {
		Self {
			position: (0, 0),
			z_order: 0, // Initially below parent (parent is 0)
		}
	}
}
impl BufferedState for SubsurfaceState {
	fn apply(&mut self, pending: &mut Self) {
		*self = pending.clone();
	}

	fn get_initial_pending(&self) -> Self {
		self.clone()
	}
}

#[derive(Debug, RequestDispatcher)]
#[waynest(error = WaylandError, connection = crate::wayland::Client)]
pub struct Subsurface {
	id: ObjectId,
	surface: Arc<Surface>,
	state: Arc<Mutex<SurfaceCommitAwareBuffer<SubsurfaceState>>>,
	child_id: Mutex<Option<u64>>,
	is_sync: AtomicBool,
}

impl Subsurface {
	pub fn new(id: ObjectId, surface: Arc<Surface>, parent: Arc<Surface>) -> Self {
		let child_id = rand::rng().random();
		let _ = surface.surface_id.set(SurfaceId::Child(child_id));
		surface.set_parent(&parent);

		Self {
			id,
			state: SurfaceCommitAwareBuffer::new(SubsurfaceState::default(), &surface),
			surface,
			child_id: Mutex::new(Some(child_id)),
			is_sync: AtomicBool::new(true), // Subsurfaces start in sync mode
		}
	}

	/// Check if this subsurface is effectively synchronized
	/// Per spec: "even if a sub-surface is set to desynchronized,
	/// a parent sub-surface may override it to behave as synchronized"
	fn is_effectively_sync(&self) -> bool {
		if !self.is_sync.load(Ordering::Acquire) {
			// We're desync, but check if parent is a synchronized subsurface
			if let Some(parent) = self.surface.parent() {
				if parent.role.get() == Some(&SurfaceRole::Subsurface) {
					// Parent is a subsurface - we inherit synchronized behavior
					// TODO: Could walk the chain recursively for perfect correctness
					return true;
				}
			}
			return false;
		}
		true
	}

	fn setup(self: &Arc<Self>) {
		// Set up commit filter to control when surface state is applied
		let subsurface_weak = Arc::downgrade(self);
		self.surface.set_parent_syncronized_filter(move || {
			let Some(subsurface) = subsurface_weak.upgrade() else {
				return true; // Subsurface gone, allow commit
			};

			subsurface.is_effectively_sync()
		});

		// First commit: add child when buffer is ready
		let subsurface_weak = Arc::downgrade(self);
		self.surface
			.add_updated_current_state_handler(move |surface| {
				let Some(subsurface) = subsurface_weak.upgrade() else {
					return true;
				};
				let Some(parent) = subsurface.surface.parent() else {
					return true;
				};
				let Some(panel_item) = parent.panel_item.lock().upgrade() else {
					return true;
				};

				if surface.currently_has_valid_buffer() {
					*surface.panel_item.lock() = Arc::downgrade(&panel_item);
					let info = subsurface.create_child_info(surface.current_buffer_size());
					panel_item.backend.add_child(&subsurface.surface, info);
					return false; // Remove handler after adding child once
				}
				true
			});
		let subsurface_weak = Arc::downgrade(self);
		self.surface.add_commit_handler(move |_| {
			let Some(subsurface) = subsurface_weak.upgrade() else {
				return true;
			};
			subsurface.state.lock().apply();
			true
		});
		// update subsurface geometry
		let subsurface_weak = Arc::downgrade(self);
		self.surface.add_updated_current_state_handler(move |_| {
			let Some(subsurface) = subsurface_weak.upgrade() else {
				return true;
			};
			let surface = subsurface.surface.clone();

			if surface.currently_has_valid_buffer() {
				if let Some(panel_item) = surface.panel_item.lock().upgrade() {
					let state = subsurface.state.lock();
					let subsurface_state = *state.current();
					drop(state);
					let size = surface
						.current_buffer_size()
						.map(|b| [b.x as u32, b.y as u32].into())
						.unwrap_or([0; 2].into());

					tracing::debug!("Updating backend after cached state apply: size={:?}", size);

					let geometry = Geometry {
						origin: [subsurface_state.position.0, subsurface_state.position.1].into(),
						size,
					};
					panel_item.backend.reposition_child(&surface, geometry);
					panel_item
						.backend
						.update_child_z_order(&surface, subsurface_state.z_order);
				}
			}
			true
		});
	}

	fn create_child_info(&self, buffer_size: Option<Vector2<usize>>) -> ChildInfo {
		let state = self.state.lock();

		let size = buffer_size
			.map(|b| [b.x as u32, b.y as u32].into())
			.unwrap_or([0; 2].into());

		// Determine parent surface ID
		let parent_surface_id = self
			.surface
			.parent()
			.and_then(|p| p.surface_id.get().cloned())
			.unwrap_or(SurfaceId::Toplevel(()));

		ChildInfo {
			id: self.child_id.lock().unwrap(),
			parent: parent_surface_id,
			geometry: Geometry {
				origin: [state.current().position.0, state.current().position.1].into(),
				size,
			},
			z_order: state.current().z_order,
			receives_input: true,
		}
	}
}

impl WlSubsurface for Subsurface {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/wayland#wl_subsurface:request:destroy
	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		// Remove the child from the parent's backend
		if let Some(parent) = self.surface.parent() {
			let Some(panel_item) = parent.panel_item.lock().upgrade() else {
				client.remove(self.id);
				return Ok(());
			};
			panel_item.backend.remove_child(&self.surface);
		}

		// Clear the commit filter
		self.surface.clear_parent_syncronized_filter();

		client.remove(self.id);
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_subsurface:request:set_position
	async fn set_position(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		x: i32,
		y: i32,
	) -> WaylandResult<()> {
		self.state.lock().pending.position = (x, y);
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_subsurface:request:place_above
	async fn place_above(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		sibling: ObjectId,
	) -> WaylandResult<()> {
		// Get the sibling's z_order
		let sibling_z_order = if let Some(sibling_surface) = client.get::<Surface>(sibling)
			&& let Some(SurfaceId::Child(sibling_id)) = sibling_surface.surface_id.get()
			&& let Some(parent) = self.surface.parent()
			&& let Some(panel_item) = parent.panel_item.lock().upgrade()
			&& let Some(child_entry) = panel_item.backend.children.get(sibling_id)
		{
			child_entry.1.z_order
		} else {
			0
		};

		// Place this subsurface one level above the sibling
		self.state.lock().pending.z_order = sibling_z_order + 1;
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_subsurface:request:place_below
	async fn place_below(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		sibling: ObjectId,
	) -> WaylandResult<()> {
		// Get the sibling's z_order
		let sibling_z_order = if let Some(sibling_surface) = client.get::<Surface>(sibling)
			&& let Some(SurfaceId::Child(sibling_id)) = sibling_surface.surface_id.get()
			&& let Some(parent) = self.surface.parent()
			&& let Some(panel_item) = parent.panel_item.lock().upgrade()
			&& let Some(child_entry) = panel_item.backend.children.get(sibling_id)
		{
			child_entry.1.z_order
		} else {
			0
		};
		// Place this subsurface one level below the sibling
		self.state.lock().pending.z_order = sibling_z_order - 1;
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_subsurface:request:set_sync
	async fn set_sync(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		self.is_sync.store(true, Ordering::Release);
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_subsurface:request:set_desync
	async fn set_desync(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		let was_sync = self.is_sync.swap(false, Ordering::AcqRel);

		if was_sync {
			// TODO: figure out if this should be recursive or only for this surface
			self.surface.update_current_state_recursive();
		}

		Ok(())
	}
}
