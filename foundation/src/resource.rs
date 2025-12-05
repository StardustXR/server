use stardust_xr_wire::values::ResourceID;
use std::{
	ffi::OsStr,
	path::{Path, PathBuf},
	sync::LazyLock,
};

static THEMES: LazyLock<Vec<PathBuf>> = LazyLock::new(|| {
	std::env::var("STARDUST_THEMES")
		.map(|s| s.split(':').map(PathBuf::from).collect())
		.unwrap_or_default()
});

fn has_extension(path: &Path, extensions: &[&OsStr]) -> bool {
	if let Some(path_extension) = path.extension() {
		extensions.contains(&path_extension)
	} else {
		false
	}
}

pub fn get_resource_file<'a>(
	resource: &ResourceID,
	base_prefixes: impl Iterator<Item = &'a PathBuf>,
	extensions: &[&OsStr],
) -> Option<PathBuf> {
	match resource {
		ResourceID::Direct(file) => {
			(file.is_absolute() && file.exists() && has_extension(file, extensions))
				.then_some(file.clone())
		}
		ResourceID::Namespaced { namespace, path } => {
			let file_name = path.file_name()?;
			THEMES
				.iter()
				.chain(base_prefixes)
				.filter_map(|prefix| {
					let prefixed_path = prefix.clone().join(namespace).join(path);
					let parent = prefixed_path.parent()?;
					std::fs::read_dir(parent).ok()
				})
				.flatten()
				.filter_map(|item| item.ok())
				.map(|dir_entry| dir_entry.path())
				.filter(|path| path.file_stem() == Some(file_name))
				.find(|path| has_extension(path, extensions))
		}
	}
}
