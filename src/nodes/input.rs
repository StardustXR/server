use super::core::{Alias, Node};
use super::field::Field;
use super::spatial::{get_spatial_parent_flex, get_transform_pose_flex, Spatial};
use crate::core::client::Client;
use crate::core::nodelist::LifeLinkedNodeList;
use crate::core::registry::Registry;
use anyhow::{anyhow, ensure, Result};
use glam::{vec3a, Mat4};
use lazy_static::lazy_static;
use libstardustxr::flex::flexbuffer_from_vector_arguments;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use parking_lot::Mutex;
use std::ops::Deref;
use std::sync::{Arc, Weak};

lazy_static! {
	static ref INPUT_METHOD_REGISTRY: Registry<InputMethod> = Default::default();
	static ref INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Default::default();
}

pub struct InputMethod {
	spatial: Weak<Spatial>,
	specialization: InputType,
}
trait InputMethodSpecialization {
	fn distance(&self, space: &Spatial, field: &Field) -> f32;
}
enum InputType {}
impl Deref for InputType {
	type Target = dyn InputMethodSpecialization;
	fn deref(&self) -> &Self::Target {
		todo!()
		// match self {
		// 	Field::Box(field) => field,
		// }
	}
}
pub struct InputHandler {
	spatial: Weak<Spatial>,
	field: Weak<Field>,
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "data", false);
	// node.add_local_signal("createInputHandler", create_input_handler_flex);
	node.add_to_scenegraph();
}
