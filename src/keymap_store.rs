use core::slice;
use std::{
	ffi::CStr,
	fs::File,
	hash::{BuildHasher, RandomState},
	io::Write,
	ops::Deref,
	os::fd::{AsFd, BorrowedFd},
	path::Path,
	ptr,
	sync::{Arc, OnceLock, Weak},
};

use dashmap::DashMap;
use gluon::{Handler, Node, RefExt, RefFsBinding};
use rustix::{
	fs::{MemfdFlags, memfd_create},
	mm::{self, MapFlags, ProtFlags, mmap},
};
use stardust_xr_protocol::keymap::{
	Keymap as KeymapProxy, KeymapExchangeError, KeymapHandler, KeymapStore as KeymapStoreProxy,
	KeymapStoreHandler, XkbcommonKeymapFd,
};
use xkbcommon_rs::{
	Context, Keymap, KeymapFormat, xkb_context::ContextFlags, xkb_keymap::CompileFlags,
};

use crate::{impl_proxy, nodes::ProxyExt};

/// The store's half of a registered keymap: the memfd it hands out, and the strong proxy
/// that keeps the keymap's node alive so every client exchanging these bytes gets the
/// same one back.
#[derive(Debug)]
struct KeymapEntry {
	file: File,
	size: u32,
	id: u64,
}
impl Drop for KeymapEntry {
	fn drop(&mut self) {
		KEYMAP_STORE.get().unwrap().map.remove(&self.id);
	}
}

#[derive(Debug, Handler)]
pub struct KeymapStore {
	map: DashMap<u64, Weak<KeymapEntry>>,
	hasher: RandomState,
}

/// A [`KeymapStore`] node published on the filesystem. Keep it for as long as the store
/// should serve — dropping it hangs the node up and unlinks the socket and its lockfile.
pub struct ExposedKeymapStore {
	node: Node<KeymapStore>,
	binding: RefFsBinding,
}
impl ExposedKeymapStore {
	pub fn path(&self) -> &Path {
		self.binding.path()
	}
}
impl Deref for ExposedKeymapStore {
	type Target = Node<KeymapStore>;
	fn deref(&self) -> &Self::Target {
		&self.node
	}
}

pub static KEYMAP_STORE: OnceLock<Arc<KeymapStore>> = OnceLock::new();
impl KeymapStore {
	pub const SERVICE_NAME: &str = "stardust-keymap-store";
	pub fn expose(instance: &str) -> std::io::Result<ExposedKeymapStore> {
		// deliberately not `create_server_file`: `RefFsBinding` takes the lockfile beside
		// the path itself, and flock is per open file description — taking it here too
		// would have this process fail to lock against its own handle
		let pion_path = stardust_xr_protocol::dir::server_file_path(Self::SERVICE_NAME, instance)
			.unwrap_or_else(|| {
				panic!(
					"failed to create {} pion dir for instance: {}",
					Self::SERVICE_NAME,
					instance
				)
			});
		let (node, keymap_store) = KeymapStoreProxy::new_node(KeymapStore {
			map: DashMap::new(),
			hasher: RandomState::new(),
		})
		.map_err(std::io::Error::other)?;
		let binding = keymap_store
			.proxy()
			.bind(&pion_path)
			.map_err(std::io::Error::other)?;
		_ = KEYMAP_STORE.set(node.handler().clone());
		Ok(ExposedKeymapStore { node, binding })
	}

	/// Register a keymap already in xkb TextV1 form (including the trailing NUL),
	/// deduplicated against previously exchanged keymaps.
	pub fn register(&self, bytes_with_nul: &[u8]) -> Result<KeymapProxy, KeymapExchangeError> {
		let hash = self.hasher.hash_one(bytes_with_nul);

		let data = if let Some(entry) = self.map.get(&hash).and_then(|v| v.upgrade()) {
			entry
		} else {
			let memfd = memfd_create("keymap", MemfdFlags::CLOEXEC).map_err(|err| {
				tracing::error!("failed to create memfd: {err}");
				KeymapExchangeError::InvalidKeymap
			})?;
			let mut file = File::from(memfd);
			file.write_all(bytes_with_nul).map_err(|err| {
				tracing::error!("failed to write to custom memfd: {err}");
				KeymapExchangeError::InvalidKeymap
			})?;

			let data = Arc::new(KeymapEntry {
				file,
				size: bytes_with_nul.len() as u32,
				id: hash,
			});
			self.map.insert(hash, Arc::downgrade(&data));
			data
		};
		let proxy = KeymapProxy::new_service(KeymapToken { id: hash, data })
			.map_err(|err| {
				tracing::error!("failed to create keymap node: {err}");
				KeymapExchangeError::InvalidKeymap
			})?
			.into_proxy();
		Ok(proxy)
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
		self.register(mmap.slice())
	}

	async fn get(&self, _ctx: gluon::Context, keymap: KeymapProxy) -> Option<XkbcommonKeymapFd> {
		let token = keymap.owned()?;
		Some(XkbcommonKeymapFd {
			fd: token.data.file.try_clone().ok()?.into(),
			size: token.data.size,
		})
	}

	async fn get_keymap_id(&self, _ctx: gluon::Context, keymap: KeymapProxy) -> Option<u64> {
		keymap.owned().map(|v| v.id)
	}
}

/// A registered keymap. Carries only its hash — everything else lives in the store's map,
/// and the store holding this token's proxy is what keeps it alive.
#[derive(Debug, Handler)]
pub struct KeymapToken {
	id: u64,
	data: Arc<KeymapEntry>,
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
