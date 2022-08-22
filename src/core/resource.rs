use std::path::PathBuf;

pub type ResourceID = Box<dyn ResourceIDTrait + Send + Sync>;

pub trait ResourceIDTrait {
	fn get_file(&self, prefixes: &[PathBuf]) -> Option<PathBuf>;
}

impl ResourceIDTrait for PathBuf {
	fn get_file(&self, _prefixes: &[PathBuf]) -> Option<PathBuf> {
		if self.is_absolute() && self.as_path().exists() {
			Some(self.clone())
		} else {
			None
		}
	}
}

pub struct NamespacedResourceID {
	pub namespace: String,
	pub path: PathBuf,
}

impl ResourceIDTrait for NamespacedResourceID {
	fn get_file(&self, prefixes: &[PathBuf]) -> Option<PathBuf> {
		for prefix in prefixes {
			let mut path = prefix.clone();
			path.push(self.namespace.clone());
			path.push(self.path.clone());

			if path.as_path().exists() {
				return Some(path);
			}
		}
		None
	}
}
