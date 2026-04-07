use std::sync::Arc;

use binderbinder::{TransactionHandler, binder_object::BinderObject};

pub mod audio;
pub mod camera;
pub mod drawable;
pub mod fields;
// pub mod input;
pub mod spatial;

/// keeps the object alive until all binder strong refs are dropped
pub fn ref_owned<H: TransactionHandler>(obj: &Arc<BinderObject<H>>) {
	let obj = obj.clone();
	tokio::spawn(async move {
		loop {
			obj.strong_refs_hit_zero().await;
			if Arc::downgrade(&obj).strong_count() == 1 {
				break;
			}
		}
	});
}
#[macro_export]
macro_rules! interface {
	($type:ident) => {
		#[derive(Debug)]
		pub struct $type {
			drop_notifs: tokio::sync::RwLock<Vec<gluon_wire::drop_tracking::DropNotifier>>,
			base_resource_prefixes: std::sync::Arc<Vec<std::path::PathBuf>>,
		}

		impl $type {
			pub fn new(base_resource_prefixes: &std::sync::Arc<Vec<std::path::PathBuf>>) -> $type {
				$type {
					drop_notifs: tokio::sync::RwLock::default(),
					base_resource_prefixes: base_resource_prefixes.clone(),
				}
			}
			fn base_prefixes(&self) -> &[std::path::PathBuf] {
				&self.base_resource_prefixes
			}
		}

		$crate::impl_transaction_handler!($type);
	};
}
#[macro_export]
macro_rules! impl_transaction_handler {
	($type:ty) => {
		impl binderbinder::TransactionHandler for $type {
			async fn handle(
				&self,
				transaction: binderbinder::device::Transaction,
			) -> binderbinder::payload::PayloadBuilder<'_> {
				let mut gluon_data = gluon_wire::GluonDataReader::from_payload(transaction.payload);
				self.dispatch_two_way(
					transaction.code,
					&mut gluon_data,
					gluon_wire::GluonCtx {
						sender_pid: transaction.sender_pid,
						sender_euid: transaction.sender_euid,
					},
				)
				.await
				.inspect_err(|err| {
					tracing::error!(
						concat!("failed to dispatch two_way {} for ", stringify!($type)),
						err
					)
				})
				.unwrap_or_else(|_| gluon_wire::GluonDataBuilder::new())
				.to_payload()
			}

			async fn handle_one_way(&self, transaction: binderbinder::device::Transaction) {
				let mut gluon_data = gluon_wire::GluonDataReader::from_payload(transaction.payload);
				_ = self
					.dispatch_one_way(
						transaction.code,
						&mut gluon_data,
						gluon_wire::GluonCtx {
							sender_pid: transaction.sender_pid,
							sender_euid: transaction.sender_euid,
						},
					)
					.await
					.inspect_err(|err| {
						tracing::error!(
							concat!("failed to dispatch one_way {} for ", stringify!($type)),
							err
						)
					});
			}
		}
	};
}
pub trait ProxyExt {
	type Owned: TransactionHandler;
	fn owned(&self) -> Option<Arc<BinderObject<Self::Owned>>>;
}
#[macro_export]
macro_rules! impl_proxy {
	($proxy:ty, $type:ty) => {
		impl crate::nodes::ProxyExt for $proxy {
			type Owned = $type;
			fn owned(&self) -> Option<Arc<BinderObject<Self::Owned>>> {
				use binderbinder::binder_object::BinderObjectOrRef;
				use binderbinder::binder_object::ToBinderObjectOrRef;
				match self.to_binder_object_or_ref() {
					BinderObjectOrRef::Object(obj) => obj.downcast(),
					// TODO: allow sending weak refs
					// should never happen with the rust version of gluon tho
					BinderObjectOrRef::WeakObject(obj) => None,
					// spatial owned by different process, this is not allowed
					BinderObjectOrRef::Ref(binder_ref) => None,
					BinderObjectOrRef::WeakRef(weak_binder_ref) => None,
				}
			}
		}
	};
}
