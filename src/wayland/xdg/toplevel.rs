use waynest::{
    server::{Client, Dispatcher, Object, Result},
    wire::ObjectId,
};

pub use waynest::server::protocol::stable::xdg_shell::xdg_toplevel::*;

#[derive(Debug, Dispatcher, Default)]
pub struct Toplevel;

impl XdgToplevel for Toplevel {
    async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
        todo!()
    }

    async fn set_parent(
        &self,
        _object: &Object,
        _client: &mut Client,
        _parent: Option<ObjectId>,
    ) -> Result<()> {
        todo!()
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
}
