#[macro_export]
macro_rules! create_interface {
	($iface:ident, $aspect:ident, $path:expr) => {
		pub fn create_interface(client: &Arc<Client>) -> Result<()> {
			let node = Node::create_path(client, $path, false);
			<$iface as $aspect>::add_node_members(&node);
			node.add_to_scenegraph()?;
			Ok(())
		}
	};
}
