#[macro_export]
macro_rules! create_interface {
	($iface:ident) => {
		pub fn create_interface(client: &Arc<Client>) -> Result<()> {
			let node = Node::from_id(client, INTERFACE_NODE_ID, false);
			<$iface as self::InterfaceAspect>::add_node_members(&node);
			node.add_to_scenegraph()?;
			Ok(())
		}
	};
}
