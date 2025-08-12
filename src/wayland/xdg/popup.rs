use super::{
	backend::XdgBackend,
	positioner::{Positioner, PositionerData},
	surface::Surface,
};
use crate::{
        nodes::items::panel::{ChildInfo, Geometry, PanelItem, SurfaceId},
        wayland::util::DoubleBuffer,
};
use parking_lot::Mutex;
use rand::Rng;
use std::{
	sync::{Arc, Weak, atomic::AtomicBool},
	u64,
};
use waynest::{
	server::{Client, Dispatcher, Result, protocol::stable::xdg_shell::xdg_popup::XdgPopup},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Popup {
	id: ObjectId,
	version: u32,
	surface_id: SurfaceId,
	parent: Arc<Surface>,
	surface: Weak<Surface>,
        pub panel_item: Weak<PanelItem<XdgBackend>>,
        positioner_data: Mutex<PositionerData>,
        geometry: Mutex<DoubleBuffer<Geometry>>,
        mapped: AtomicBool,
}
impl Popup {
	pub fn new(
		id: ObjectId,
		version: u32,
		parent: Arc<Surface>,
		panel_item: &Arc<PanelItem<XdgBackend>>,
		xdg_surface: &Arc<Surface>,
		positioner: &Positioner,
	) -> Self {
		let positioner_data = positioner.data();
		Self {
			id,
			version,
			surface_id: SurfaceId::Child(rand::thread_rng().gen_range(0..u64::MAX)),
			parent,
			surface: Arc::downgrade(xdg_surface),
                        panel_item: Arc::downgrade(panel_item),
                        positioner_data: Mutex::new(positioner_data),
                        geometry: Mutex::new(DoubleBuffer::new(positioner_data.infinite_geometry())),
                        mapped: AtomicBool::new(false),
                }
        }
}

impl Popup {
        fn id(&self) -> u64 {
                match self.surface_id {
                        SurfaceId::Child(id) => id,
                        SurfaceId::Toplevel(_) => 0,
                }
        }

        pub fn surface_id(&self) -> SurfaceId { self.surface_id.clone() }

        pub fn current_geometry(&self) -> Geometry {
                *self.geometry.lock().current()
        }

        pub fn is_mapped(&self) -> bool {
                self.mapped.load(std::sync::atomic::Ordering::SeqCst)
        }

        pub fn map(&self) {
                if self.is_mapped() {
                        return;
                }

                let Some(panel_item) = self.panel_item.upgrade() else { return; };
                let xdg_surface = match self.surface.upgrade() { Some(s) => s, None => return };
                let core_surface = xdg_surface.wl_surface();

                // Determine parent surface id
                let parent_wl_surface = self.parent.wl_surface();
                let parent_role = parent_wl_surface.role.lock();
                let parent_id = match parent_role.as_ref().unwrap() {
                        crate::wayland::core::surface::SurfaceRole::XdgToplevel(_) => {
                                SurfaceId::Toplevel(())
                        }
                        crate::wayland::core::surface::SurfaceRole::XDGPopup(p) => p.surface_id(),
                };

                let geometry = *self.geometry.lock().current();
                let info = ChildInfo {
                        id: self.id(),
                        parent: parent_id,
                        geometry,
                        z_order: 0,
                        receives_input: true,
                };
                panel_item.create_child(self.id(), &info);
                panel_item.backend.register_child(self.id(), &core_surface);
                self.mapped.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        pub fn unmap(&self) {
                if !self.mapped.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        return;
                }

                if let Some(panel_item) = self.panel_item.upgrade() {
                        panel_item.destroy_child(self.id());
                        panel_item.backend.unregister_child(self.id());
                }
        }

        fn reposition_child(&self) {
                if self.is_mapped() {
                        if let Some(panel_item) = self.panel_item.upgrade() {
                                let geometry = *self.geometry.lock().current();
                                panel_item.reposition_child(self.id(), &geometry);
                        }
                }
        }
}
impl XdgPopup for Popup {
	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:grab
	async fn grab(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
	) -> Result<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:reposition
	async fn reposition(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		positioner: ObjectId,
		token: u32,
	) -> Result<()> {
                let positioner = client.get::<Positioner>(positioner).unwrap();
                let positioner_data = positioner.data();
                *self.positioner_data.lock() = positioner_data;
                if self.version >= 5 {
                        self.repositioned(client, sender_id, token).await?;
                }
                let geometry = positioner_data.infinite_geometry();
                {
                        let mut geo = self.geometry.lock();
                        geo.pending = geometry;
                        geo.apply();
                }
                self.configure(
                        client,
                        sender_id,
                        geometry.origin.x,
                        geometry.origin.y,
                        geometry.size.x as i32,
                        geometry.size.y as i32,
                )
                .await?;
                self.surface.upgrade().unwrap().reconfigure(client).await?;
                self.reposition_child();
                Ok(())
        }

	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:destroy
        async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
                self.unmap();
                Ok(())
        }
}
