use binderbinder::{TransactionHandler, binder_object::BinderObjectRef};

pub mod audio;
pub mod camera;
pub mod drawable;
pub mod fields;
pub mod spatial;

#[macro_export]
macro_rules! interface {
	($type:ident) => {
		#[derive(Debug, gluon_wire::Handler)]
		pub struct $type {
			base_resource_prefixes: std::sync::Arc<Vec<std::path::PathBuf>>,
		}

		impl $type {
			pub fn new(
				base_resource_prefixes: &std::sync::Arc<Vec<std::path::PathBuf>>,
			) -> binderbinder::binder_object::BinderObject<$type> {
				$crate::PION.register_object($type {
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
		#[derive(Debug, gluon_wire::Handler)]
		pub struct $type {
			_lock: std::fs::File,
			pub pion_path: std::path::PathBuf,
		}

		impl $type {
			pub async fn expose(
				instance: &str,
			) -> binderbinder::binder_object::BinderObject<$type> {
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
pub trait ProxyExt {
	type Owned: TransactionHandler;
	fn owned(&self) -> Option<BinderObjectRef<Self::Owned>>;
}
#[macro_export]
macro_rules! impl_proxy {
	($proxy:ty, $type:ty) => {
		impl $crate::nodes::ProxyExt for $proxy {
			type Owned = $type;
			fn owned(&self) -> Option<binderbinder::binder_object::BinderObjectRef<Self::Owned>> {
				use binderbinder::binder_object::BinderObjectOrRef;
				use binderbinder::binder_object::ToBinderObjectOrRef;
				match self.to_binder_object_or_ref() {
					BinderObjectOrRef::Object(obj) => obj.downcast::<Self::Owned>(),
					// TODO: allow sending weak refs
					// should never happen with the rust version of gluon tho
					BinderObjectOrRef::WeakObject(_obj) => None,
					// spatial owned by different process, this is not allowed
					BinderObjectOrRef::Ref(_binder_ref) => None,
					BinderObjectOrRef::WeakRef(_weak_binder_ref) => None,
				}
			}
		}
	};
}
