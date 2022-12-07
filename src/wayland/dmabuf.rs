use super::state::WaylandState;
use smithay::{
	backend::allocator::dmabuf::Dmabuf,
	delegate_dmabuf,
	wayland::dmabuf::{self, DmabufGlobal, DmabufHandler, DmabufState},
};

impl DmabufHandler for WaylandState {
	fn dmabuf_state(&mut self) -> &mut DmabufState {
		&mut self.dmabuf_state
	}

	fn dmabuf_imported(
		&mut self,
		_global: &DmabufGlobal,
		dmabuf: Dmabuf,
	) -> Result<(), dmabuf::ImportError> {
		self.pending_dmabufs.push(dmabuf);
		Ok(())
	}
}
delegate_dmabuf!(WaylandState);
