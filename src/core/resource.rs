use anyhow::anyhow;
use serde::{de::Visitor, Deserialize};
use std::path::PathBuf;

pub enum ResourceID {
	File(PathBuf),
	Namespaced { namespace: String, path: PathBuf },
}
impl ResourceID {
	pub fn get_file(&self, prefixes: &[PathBuf]) -> Option<PathBuf> {
		match self {
			ResourceID::File(file) => (file.is_absolute() && file.exists()).then_some(file.clone()),
			ResourceID::Namespaced { namespace, path } => {
				for prefix in prefixes {
					let mut test_path = prefix.clone();
					test_path.push(namespace.clone());
					test_path.push(path.clone());

					if test_path.as_path().exists() {
						return Some(test_path);
					}
				}
				None
			}
		}
	}
}
impl<'de> Deserialize<'de> for ResourceID {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		deserializer.deserialize_any(ResourceVisitor)
	}
}

struct ResourceVisitor;
impl<'de> Visitor<'de> for ResourceVisitor {
	type Value = ResourceID;

	fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
		formatter.write_str("A string containing an absolute path to file or \"[namespace]:[path]\" for a namespaced resource")
	}

	fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
	where
		E: serde::de::Error,
	{
		Ok(if v.starts_with('/') {
			let path = PathBuf::from(v);
			path.metadata().map_err(serde::de::Error::custom)?;
			ResourceID::File(path)
		} else if let Some((namespace, path)) = v.split_once(':') {
			ResourceID::Namespaced {
				namespace: namespace.to_string(),
				path: PathBuf::from(path),
			}
		} else {
			return Err(serde::de::Error::custom(anyhow!(
				"Invalid format for string"
			)));
		})
	}

	fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
	where
		E: serde::de::Error,
	{
		self.visit_str(&v)
	}
}
