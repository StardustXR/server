use anyhow::bail;
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

pub fn parse_resource_id(reader: flexbuffers::Reader<&[u8]>) -> anyhow::Result<ResourceID> {
	let string = reader.get_str()?;

	Ok(if string.starts_with('/') {
		let path = PathBuf::from(string);
		path.metadata()?;
		ResourceID::File(path)
	} else if let Some((namespace, path)) = string.split_once(':') {
		ResourceID::Namespaced {
			namespace: namespace.to_string(),
			path: PathBuf::from(path),
		}
	} else {
		bail!("Invalid format for string");
	})
}
