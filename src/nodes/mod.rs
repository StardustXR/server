use gluon::Handler;

pub mod audio;
// FIX ORDER: 2
// pub mod camera;
pub mod drawable;
pub mod fields;
pub mod spatial;

#[macro_export]
macro_rules! interface {
	($type:ident) => {
		#[derive(Debug, gluon::Handler)]
		pub struct $type {
			base_resource_prefixes: std::sync::Arc<Vec<std::path::PathBuf>>,
		}

		impl $type {
			pub fn new(
				base_resource_prefixes: &std::sync::Arc<Vec<std::path::PathBuf>>,
			) -> Result<(gluon::Node<$type>, gluon::Ref), gluon::NodeError> {
				gluon::Node::new($type {
					base_resource_prefixes: base_resource_prefixes.clone(),
				})
			}
			#[allow(unused)]
			fn base_prefixes(&self) -> &[std::path::PathBuf] {
				&self.base_resource_prefixes
			}
		}
	};
}
#[macro_export]
macro_rules! exposed_interface {
	($type:ident, $service:literal) => {
		#[derive(Debug, gluon::Handler)]
		pub struct $type {
			_lock: std::fs::File,
			pub pion_path: std::path::PathBuf,
		}

		impl $type {
			pub async fn expose(instance: &str) -> gluon::Object<$type> {
				let (pion_path, lock) = stardust_xr_protocol::dir::create_pion_file(
					$service, &instance,
				)
				.expect(&format!(
					"failed to create {} pion file for instance: {}",
					$service, instance,
				));
				let pion_file = std::fs::OpenOptions::new()
					.create(true)
					.read(true)
					.write(true)
					.open(&pion_path)
					.expect("failed to open file even tho we're holding a lock file for it");
				let interface = $crate::PION.register_object($type {
					_lock: lock,
					pion_path,
				});
				$crate::PION
					.bind_binder_ref_to_file(pion_file, &interface)
					.await
					.expect(&format!(
						"failed to register {} with pion",
						stringify!($type)
					));
				interface
			}
		}
	};
}
/// Recovering the handler behind a proxy this process is itself serving.
///
/// Under strong-ipc a proxy is a `Ref` — a send capability, nothing more — so this is a
/// lookup in gluon's local-handler registry rather than a downcast. `None` covers every
/// way it can fail without distinguishing them: the ref leads to another process, its node
/// is gone, or it is live and simply isn't the type we asked for.
pub trait ProxyExt {
	type Owned: Handler;
	fn owned(&self) -> Option<std::sync::Arc<Self::Owned>>;
}
#[macro_export]
macro_rules! impl_proxy {
	($proxy:ty, $type:ty) => {
		impl $crate::nodes::ProxyExt for $proxy {
			type Owned = $type;
			fn owned(&self) -> Option<std::sync::Arc<$type>> {
				gluon::RefExt::local_handler::<$type>(self)
			}
		}
	};
}
