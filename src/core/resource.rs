use color_eyre::eyre::eyre;
use serde::{de::Visitor, Deserialize};
use std::{ffi::OsStr, path::PathBuf};

pub enum ResourceID {
	File(PathBuf),
	Namespaced { namespace: String, path: PathBuf },
}
impl ResourceID {
	pub fn get_file(&self, prefixes: &[PathBuf], extensions: &[&OsStr]) -> Option<PathBuf> {
		match self {
			ResourceID::File(file) => (file.is_absolute()
				&& file.exists() && Self::has_extension(file, extensions))
			.then_some(file.clone()),
			ResourceID::Namespaced { namespace, path } => {
				let file_name = path.file_name()?;
				prefixes
					.iter()
					.filter_map(|prefix| {
						let prefixed_path = prefix.clone().join(namespace).join(path);
						let parent = prefixed_path.parent()?;
						std::fs::read_dir(parent).ok()
					})
					.flatten()
					.filter_map(|item| item.ok())
					.map(|dir_entry| dir_entry.path())
					.filter(|path| path.file_stem() == Some(file_name))
					.find(|path| Self::has_extension(path, extensions))
			}
		}
	}

	fn has_extension(path: &PathBuf, extensions: &[&OsStr]) -> bool {
		if let Some(path_extension) = path.extension() {
			extensions.contains(&path_extension)
		} else {
			false
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
			return Err(serde::de::Error::custom(eyre!("Invalid format for string")));
		})
	}

	fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
	where
		E: serde::de::Error,
	{
		self.visit_str(&v)
	}
}
