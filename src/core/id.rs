use serde::{Deserialize, Serialize, Serializer};
use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(pub u64);
impl From<u64> for Id {
	fn from(id: u64) -> Self {
		Self(id)
	}
}
impl Display for Id {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}
impl<'de> Deserialize<'de> for Id {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		u64::deserialize(deserializer).map(Id)
	}
}
impl Serialize for Id {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.0.serialize(serializer)
	}
}
