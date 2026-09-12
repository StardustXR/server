use gluon_ipc::Handler;

pub mod audio;
pub mod camera;
pub mod drawable;
pub mod fields;
pub mod spatial;

#[macro_export]
macro_rules! interface {
	($type:ident) => {
		#[derive(Debug, gluon_ipc::Handler)]
		pub struct $type {
			base_resource_prefixes: std::sync::Arc<Vec<std::path::PathBuf>>,
		}

		impl $type {
			pub fn new(base_resource_prefixes: &std::sync::Arc<Vec<std::path::PathBuf>>) -> Self {
				$type {
					base_resource_prefixes: base_resource_prefixes.clone(),
				}
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
		#[derive(Debug, gluon_ipc::Handler)]
		pub struct $type {
			ref_binding: std::sync::OnceLock<gluon_ipc::RefFsBinding>,
		}

		impl $type {
			pub async fn expose(instance: &str) -> gluon_ipc::Node<$type> {
				let path = stardust_xr_protocol::dir::server_file_path($service, instance).expect(
					&format!("failed to get {} path for instance: {}", $service, instance,),
				);
				let (node, node_ref) = gluon_ipc::Node::new($type {
					ref_binding: std::sync::OnceLock::new(),
				})
				.expect(&format!("failed to create node for {}", stringify!($type)));
				let fs_binding = gluon_ipc::RefFsBinding::new(node_ref, path).expect(&format!(
					"failed to create node for {}: {}",
					$service, instance
				));
				_ = node.ref_binding.set(fs_binding);

				node
			}
			pub fn path(&self) -> &std::path::Path {
				self.ref_binding.get().unwrap().path()
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
pub trait ProxyExt: Sized {
	type Owned: Handler;
	fn owned(&self) -> Option<std::sync::Arc<Self::Owned>>;
	fn owned_ref(&self) -> Option<gluon_ipc::LocalRef<Self, Self::Owned>>;
}
#[macro_export]
macro_rules! impl_proxy {
	($proxy:ty, $type:ty) => {
		impl $crate::nodes::ProxyExt for $proxy {
			type Owned = $type;
			fn owned(&self) -> Option<std::sync::Arc<$type>> {
				gluon_ipc::RefExt::local_handler::<$type>(self)
			}
			fn owned_ref(&self) -> Option<gluon_ipc::LocalRef<$proxy, $type>> {
				Some(gluon_ipc::LocalRef::new(self.clone(), self.owned()?))
			}
		}
	};
}
