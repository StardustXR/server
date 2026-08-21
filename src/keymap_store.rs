use core::slice;
use std::{
	ffi::CStr,
	fs::File,
	hash::{BuildHasher, RandomState},
	io::Write,
	os::fd::{AsFd, BorrowedFd},
	path::PathBuf,
	ptr,
	sync::{Arc, OnceLock},
};

use dashmap::DashMap;
use gluon::{Handler, RefExt};
use rustix::{
	fs::{MemfdFlags, memfd_create},
	mm::{self, MapFlags, ProtFlags, mmap},
};
use stardust_xr_protocol::keymap::{
	Keymap as KeymapProxy, KeymapExchangeError, KeymapHandler, KeymapStoreHandler,
	XkbcommonKeymapFd,
};
use xkbcommon_rs::{
	Context, Keymap, KeymapFormat, xkb_context::ContextFlags, xkb_keymap::CompileFlags,
};

use crate::{PION, impl_proxy, nodes::ProxyExt};

#[derive(Debug, Handler)]
pub struct KeymapStore {
	map: Arc<DashMap<u64, (File, u32, WeakBinderObject)>>,
	_lock: File,
	hasher: RandomState,
	pub pion_path: PathBuf,
}

pub static KEYMAP_STORE: OnceLock<Arc<KeymapStore>> = OnceLock::new();
impl KeymapStore {
	pub const SERVICE_NAME: &str = "stardust-keymap-store";
	pub async fn expose(instance: &str) -> gluon::Object<Self> {
		let (pion_path, lock) =
			stardust_xr_protocol::dir::create_server_file(Self::SERVICE_NAME, instance)
				.unwrap_or_else(|| {
					panic!(
						"failed to create {} pion file for instance: {}",
						Self::SERVICE_NAME,
						instance
					)
				});
		let pion_file = std::fs::OpenOptions::new()
			.create(true)
			.truncate(false)
			.read(true)
			.write(true)
			.open(&pion_path)
			.expect("failed to open file even tho we're holding a lock file for it");
		let interface = PION.register_object(KeymapStore {
			map: Default::default(),
			_lock: lock,
			hasher: RandomState::new(),
			pion_path,
		});
		PION.bind_binder_ref_to_file(pion_file, &interface)
			.await
			.unwrap_or_else(|_| panic!("failed to register {} with pion", stringify!($type)));
		_ = KEYMAP_STORE.set(interface.clone());
		interface
	}

	/// Register a keymap already in xkb TextV1 form (including the trailing NUL),
	/// deduplicated against previously exchanged keymaps. The token stays alive as
	/// long as the returned proxy is held.
	pub fn register_keymap_bytes(
		&self,
		bytes_with_nul: &[u8],
	) -> Result<KeymapProxy, KeymapExchangeError> {
		let hash = self.hasher.hash_one(bytes_with_nul);
		// if let Some(binder_obj) = self.map.get(&hash).map(|v| v.value().2.clone())
		// 	&& let Some(binder_obj) = binder_obj.upgrade()
		// 	&& let Some(keymap) = binder_obj.downcast::<KeymapToken>()
		// {
		// 	return Ok(KeymapProxy::from_handler(&keymap));
		// }
		let memfd = memfd_create("keymap", MemfdFlags::CLOEXEC).map_err(|err| {
			tracing::error!("failed to create memfd: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		// TODO: important, dedup using /proc/net/unix refcount
		let mut file = File::from(memfd);
		file.write_all(bytes_with_nul).map_err(|err| {
			tracing::error!("failed to write to custom memfd: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		let keymap_obj = KeymapProxy::new_service(KeymapToken {
			id: hash,
			map: self.map.clone(),
		})
		.map_err(|err| {
			tracing::error!("failed to create keymap node: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		// self.map.insert(
		// 	hash,
		// 	(file, bytes_with_nul.len() as u32, keymap_obj.downgrade()),
		// );
		Ok(keymap_obj)
	}
}
impl KeymapStoreHandler for KeymapStore {
	async fn exchange(
		&self,
		_ctx: gluon::Context,
		keymap: XkbcommonKeymapFd,
	) -> Result<KeymapProxy, KeymapExchangeError> {
		let mmap = MMapGuard::new(keymap.fd.as_fd(), keymap.size as usize).map_err(|err| {
			tracing::error!("failed to map fd {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		let cstr = CStr::from_bytes_with_nul(mmap.slice()).map_err(|err| {
			tracing::error!("failed to create keymap cstr: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		let str = cstr.to_str().map_err(|err| {
			tracing::error!("failed to get keymap str: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		Keymap::new_from_string(
			Context::new(ContextFlags::empty()).map_err(|err| {
				tracing::error!("failed to create keymap ctx: {err}");
				KeymapExchangeError::InvalidKeymap
			})?,
			str,
			KeymapFormat::TextV1,
			CompileFlags::empty(),
		)
		.map_err(|err| {
			tracing::error!("invalid keymap: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		self.register_keymap_bytes(mmap.slice())
	}

	async fn get(&self, _ctx: gluon::Context, keymap: KeymapProxy) -> Option<XkbcommonKeymapFd> {
		let token = keymap.owned()?;
		let v = self.map.get(&token.id)?;
		let (file, size, _) = v.value();
		Some(XkbcommonKeymapFd {
			fd: file.try_clone().ok()?.into(),
			size: *size,
		})
	}
}
#[derive(Debug, Handler)]
pub struct KeymapToken {
	id: u64,
	map: Arc<DashMap<u64, (File, u32, WeakBinderObject)>>,
}
impl Drop for KeymapToken {
	fn drop(&mut self) {
		self.map.remove(&self.id);
	}
}
impl KeymapHandler for KeymapToken {}
impl_proxy!(KeymapProxy, KeymapToken);

struct MMapGuard(&'static mut [u8]);
impl MMapGuard {
	fn new(fd: BorrowedFd, size: usize) -> rustix::io::Result<Self> {
		let slice = unsafe {
			let ptr = mmap(
				ptr::null_mut(),
				size,
				ProtFlags::READ,
				MapFlags::PRIVATE,
				fd,
				0,
			)?;
			slice::from_raw_parts_mut(ptr.cast::<u8>(), size)
		};
		Ok(Self(slice))
	}
	fn slice(&self) -> &[u8] {
		self.0
	}
}
impl Drop for MMapGuard {
	fn drop(&mut self) {
		unsafe {
			_ = mm::munmap(self.0.as_mut_ptr().cast(), self.0.len())
				.inspect_err(|err| tracing::error!("failed to unmap keymap fd: {err}"));
		}
	}
}
