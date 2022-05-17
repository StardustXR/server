use super::core::Node;

pub struct Spatial<'a> {
	node: &'a Node<'a>,
}

impl<'a> Spatial<'a> {
	pub fn new(node: &'a Node<'a>) -> Self {
		Spatial { node }
	}
}
