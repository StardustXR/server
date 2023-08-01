#![allow(unused)]

use std::ops::{Deref, DerefMut};

use color_eyre::eyre::Result;
use once_cell::sync::Lazy;
use serde::{de::DeserializeOwned, Serialize};
use stardust_xr::schemas::{
	flat::Datamap,
	flex::flexbuffers::{FlexbufferSerializer, Reader, ReaderError},
};

use crate::nodes::Message;

pub struct TypedDatamap<T: DeserializeOwned + Serialize>(T);
impl<T: DeserializeOwned + Serialize> TypedDatamap<T> {
	pub fn new(data: T) -> Self {
		TypedDatamap(data)
	}
	pub fn from_flex(message: Message) -> Result<Self> {
		let root = Reader::get_root(message.as_ref())?;
		T::deserialize(root).map(Self::new).map_err(|e| e.into())
	}
	pub fn to_datamap(&mut self) -> Result<Datamap> {
		let mut serializer = FlexbufferSerializer::default();
		self.0.serialize(&mut serializer)?;
		Datamap::new(serializer.take_buffer()).map_err(|e| e.into())
	}
	pub fn serialize(&mut self) -> Option<Vec<u8>> {
		let mut serializer = FlexbufferSerializer::default();
		self.0.serialize(&mut serializer).ok()?;
		// check if this is actually a map
		Reader::get_root(serializer.view()).ok()?.get_map().ok()?;
		Some(serializer.take_buffer())
	}
}
impl<T: DeserializeOwned + Serialize> Default for TypedDatamap<T>
where
	T: Default,
{
	fn default() -> Self {
		Self(T::default())
	}
}
impl<T: DeserializeOwned + Serialize> Deref for TypedDatamap<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}
impl<T: DeserializeOwned + Serialize> DerefMut for TypedDatamap<T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.0
	}
}
