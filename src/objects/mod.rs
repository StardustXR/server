#![allow(unused)]

use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		Node, OwnedNode,
		fields::{EXPORTED_FIELDS, Field, Shape},
		spatial::{EXPORTED_SPATIALS, Spatial},
	},
};
use glam::{Mat4, vec3};
use input::{
	eye_pointer::EyePointer, mouse_pointer::MousePointer, oxr_controller::OxrControllerInput,
	oxr_hand::OxrHandInput,
};
use parking_lot::RwLock;
use play_space::PlaySpaceBounds;
use stardust_xr_gluon::object_registry::ObjectRegistry;
use std::{
	marker::PhantomData,
	sync::{Arc, atomic::Ordering},
};
use tokio::{sync::mpsc, task::AbortHandle};
use zbus::{Connection, interface, object_server::Interface, zvariant::OwnedObjectPath};

pub mod hmd;
pub mod input;
pub mod play_space;

pub struct ObjectHandle<I: Interface>(Connection, OwnedObjectPath, PhantomData<I>);

impl<I: Interface> Clone for ObjectHandle<I> {
	fn clone(&self) -> Self {
		Self(self.0.clone(), self.1.clone(), PhantomData)
	}
}
impl<I: Interface> Drop for ObjectHandle<I> {
	fn drop(&mut self) {
		let connection = self.0.clone();
		let object_path = self.1.clone();
		tokio::task::spawn(async move {
			connection.object_server().remove::<I, _>(object_path);
		});
	}
}

/// A wrapper around ObjectHandle<Tracked> that batches async updates
/// instead of spawning a tokio task for each state change
pub struct AsyncTracked {
	pub sender: mpsc::UnboundedSender<bool>,
	pub _handle: ObjectHandle<Tracked>,
	pub _abort_handle: AbortHandle,
}

impl AsyncTracked {
	pub fn new(connection: &Connection, path: &str) -> Self {
		let handle = Tracked::new(connection, path);
		let (sender, mut receiver) = mpsc::unbounded_channel::<bool>();

		// Spawn a single long-running task that processes state updates
		let task = tokio::task::spawn({
			let handle = handle.clone();
			async move {
				while let Some(is_tracked) = receiver.recv().await {
					let _ = handle.set_tracked(is_tracked).await;
				}
			}
		});

		Self {
			sender,
			_handle: handle,
			_abort_handle: task.abort_handle(),
		}
	}

	pub fn set_tracked(&self, is_tracked: bool) {
		// Just send over channel instead of spawning a task
		let _ = self.sender.send(is_tracked);
	}
}

impl Drop for AsyncTracked {
	fn drop(&mut self) {
		self._abort_handle.abort();
	}
}

pub struct SpatialRef(u64, OwnedNode);
impl SpatialRef {
	pub fn create(connection: &Connection, path: &str) -> (Arc<Spatial>, ObjectHandle<SpatialRef>) {
		let node = OwnedNode(Arc::new(Node::generate(&INTERNAL_CLIENT, false)));
		let spatial = Spatial::add_to(&node.0, None, Mat4::IDENTITY);
		let uid: u64 = rand::random();
		EXPORTED_SPATIALS
			.lock()
			.insert(uid, Arc::downgrade(&node.0));

		tokio::task::spawn({
			let connection = connection.clone();
			let path = path.to_string();
			async move {
				connection
					.object_server()
					.at(path, Self(uid, node))
					.await
					.unwrap();
			}
		});
		(
			spatial,
			ObjectHandle(
				connection.clone(),
				OwnedObjectPath::try_from(path.to_string()).unwrap(),
				PhantomData,
			),
		)
	}
}
#[interface(name = "org.stardustxr.SpatialRef")]
impl SpatialRef {
	#[zbus(property)]
	fn uid(&self) -> u64 {
		self.0
	}
}

pub struct Tracked(bool);
impl Tracked {
	pub fn new(connection: &Connection, path: &str) -> ObjectHandle<Tracked> {
		tokio::task::spawn({
			let connection = connection.clone();
			let path = path.to_string();
			async move {
				connection
					.object_server()
					.at(path, Self(false))
					.await
					.unwrap();
			}
		});
		ObjectHandle(
			connection.clone(),
			OwnedObjectPath::try_from(path.to_string()).unwrap(),
			PhantomData,
		)
	}
}
impl ObjectHandle<Tracked> {
	pub async fn set_tracked(&self, is_tracked: bool) -> zbus::Result<()> {
		let tracked_ref = self
			.0
			.object_server()
			.interface::<_, Tracked>(self.1.as_ref())
			.await?;
		let mut tracked = tracked_ref.get_mut().await;
		if tracked.0 != is_tracked {
			tracked.0 = is_tracked;
			tracked
				.is_tracked_changed(tracked_ref.signal_emitter())
				.await;
		}
		Ok(())
	}
}
#[interface(name = "org.stardustxr.Tracked")]
impl Tracked {
	#[zbus(property)]
	fn is_tracked(&self) -> bool {
		self.0
	}
}

pub struct FieldRef(u64, OwnedNode);
impl FieldRef {
	pub fn create(
		connection: &Connection,
		path: &str,
		shape: Shape,
	) -> (Arc<Field>, ObjectHandle<FieldRef>) {
		let node = OwnedNode(Arc::new(Node::generate(&INTERNAL_CLIENT, false)));
		Spatial::add_to(&node.0, None, Mat4::IDENTITY);
		let field = Field::add_to(&node.0, shape).unwrap();
		let uid: u64 = rand::random();
		EXPORTED_FIELDS.lock().insert(uid, Arc::downgrade(&node.0));

		tokio::task::spawn({
			let connection = connection.clone();
			let path = path.to_string();
			async move {
				connection
					.object_server()
					.at(path, Self(uid, node))
					.await
					.unwrap();
			}
		});
		(
			field,
			ObjectHandle(
				connection.clone(),
				OwnedObjectPath::try_from(path.to_string()).unwrap(),
				PhantomData,
			),
		)
	}
}
#[interface(name = "org.stardustxr.FieldRef")]
impl FieldRef {
	#[zbus(property)]
	fn uid(&self) -> u64 {
		self.0
	}
}
