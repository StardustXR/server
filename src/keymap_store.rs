use core::slice;
use std::{
	collections::HashSet,
	ffi::CStr,
	fs::File,
	hash::{BuildHasher, RandomState},
	io::Write,
	ops::Deref,
	os::fd::{AsFd, BorrowedFd},
	path::PathBuf,
	ptr,
	sync::{Arc, OnceLock},
	time::{Duration, Instant},
};

use dashmap::DashMap;
use gluon::{Handler, Node, RefExt, RefFsBinding, ToRef};
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
	proxy: KeymapProxy,
	/// The proxy's socket inode, for asking `/proc` whether anyone else still holds it.
	/// `None` if the `fstat` failed, which makes this entry permanently un-prunable
	/// rather than wrongly collectable.
	socket_inode: Option<u64>,
	/// Registered by the server itself rather than exchanged by a client, so an
	/// in-process consumer is holding the proxy and [`KeymapStore::prune`] must leave it
	/// alone. It cannot tell: gluon interns refs per socket, so a proxy cached in this
	/// process shares one descriptor with the store's own copy and is invisible to a
	/// walk of `/proc`.
	pinned: bool,
	/// When this entry was made, so a keymap cannot be collected in the window between
	/// `exchange` returning it and the reply actually reaching the client that asked.
	created: Instant,
}

/// How long a freshly exchanged keymap is safe from [`KeymapStore::prune`], covering the
/// gap between the handler returning a proxy and the client having a descriptor for it.
const PRUNE_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, Handler)]
pub struct KeymapStore {
	map: DashMap<u64, KeymapEntry>,
	hasher: RandomState,
	pub pion_path: PathBuf,
}

/// A [`KeymapStore`] node published on the filesystem. Keep it for as long as the store
/// should serve — dropping it hangs the node up and unlinks the socket and its lockfile.
pub struct ExposedKeymapStore {
	node: Node<KeymapStore>,
	_binding: RefFsBinding,
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
		let (node, proxy) = KeymapStoreProxy::new_node(KeymapStore {
			map: DashMap::new(),
			hasher: RandomState::new(),
			pion_path: pion_path.clone(),
		})
		.map_err(std::io::Error::other)?;
		let binding = proxy.bind(&pion_path).map_err(std::io::Error::other)?;
		_ = KEYMAP_STORE.set(node.handler().clone());
		Ok(ExposedKeymapStore {
			node,
			_binding: binding,
		})
	}

	/// Register a keymap already in xkb TextV1 form (including the trailing NUL),
	/// deduplicated against previously exchanged keymaps.
	///
	/// For the server's own keymaps. The result is pinned for the store's lifetime,
	/// because a caller in this process caching the proxy — which is exactly what
	/// [`crate::objects::input::mouse_pointer`] does — is something `prune` cannot see.
	pub fn register_keymap_bytes(
		&self,
		bytes_with_nul: &[u8],
	) -> Result<KeymapProxy, KeymapExchangeError> {
		self.register(bytes_with_nul, true)
	}

	fn register(
		&self,
		bytes_with_nul: &[u8],
		pinned: bool,
	) -> Result<KeymapProxy, KeymapExchangeError> {
		self.prune();

		let hash = self.hasher.hash_one(bytes_with_nul);
		if let Some(mut entry) = self.map.get_mut(&hash) {
			// a client exchanging the same bytes as a server keymap gets the pinned one,
			// and a server registration of bytes a client got here first pins those
			entry.pinned |= pinned;
			return Ok(entry.proxy.clone());
		}

		let memfd = memfd_create("keymap", MemfdFlags::CLOEXEC).map_err(|err| {
			tracing::error!("failed to create memfd: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		let mut file = File::from(memfd);
		file.write_all(bytes_with_nul).map_err(|err| {
			tracing::error!("failed to write to custom memfd: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		let proxy = KeymapProxy::new_service(KeymapToken { id: hash }).map_err(|err| {
			tracing::error!("failed to create keymap node: {err}");
			KeymapExchangeError::InvalidKeymap
		})?;
		let socket_inode = socket_inode(proxy.to_ref().as_fd());
		self.map.insert(
			hash,
			KeymapEntry {
				file,
				size: bytes_with_nul.len() as u32,
				proxy: proxy.clone(),
				socket_inode,
				pinned,
				created: Instant::now(),
			},
		);
		Ok(proxy)
	}

	/// Drop every keymap nothing outside this process still holds, except the pinned ones
	/// and the ones too young to have reached their client yet.
	///
	/// This is what stands in for a weak reference. The store has to keep a strong proxy
	/// to hand the same keymap back to the next client that exchanges the same bytes, and
	/// holding it is exactly what keeps the node alive — so `Ref::is_dead` can never fire
	/// and the question "does anyone *else* have this?" has to go to the kernel. It goes
	/// to `/proc/<pid>/fd`, which is the only place that answers it: `/proc/net/unix`'s
	/// RefCount column counts references to the `struct sock`, not descriptors, and does
	/// not move when a socket is duplicated or passed to another process.
	///
	/// Fails open in every direction — a keymap is a few KiB of memfd, and handing a
	/// client a dead proxy because `/proc` was unreadable is the worse outcome.
	fn prune(&self) {
		let Some(foreign) = foreign_socket_inodes() else {
			return;
		};
		self.map.retain(|_, entry| {
			entry.pinned
				|| entry.created.elapsed() < PRUNE_GRACE
				|| entry
					.socket_inode
					.is_none_or(|inode| foreign.contains(&inode))
		});
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
		self.register(mmap.slice(), false)
	}

	async fn get(&self, _ctx: gluon::Context, keymap: KeymapProxy) -> Option<XkbcommonKeymapFd> {
		let token = keymap.owned()?;
		let entry = self.map.get(&token.id)?;
		Some(XkbcommonKeymapFd {
			fd: entry.file.try_clone().ok()?.into(),
			size: entry.size,
		})
	}
}

/// A registered keymap. Carries only its hash — everything else lives in the store's map,
/// and the store holding this token's proxy is what keeps it alive.
#[derive(Debug, Handler)]
pub struct KeymapToken {
	id: u64,
}
impl KeymapHandler for KeymapToken {}
impl_proxy!(KeymapProxy, KeymapToken);

/// The inode naming this socket, which is what `/proc/<pid>/fd` links report it as.
fn socket_inode(fd: BorrowedFd) -> Option<u64> {
	rustix::fs::fstat(fd)
		.inspect_err(|err| tracing::error!("failed to stat keymap proxy socket: {err}"))
		.ok()
		.map(|stat| stat.st_ino)
}

/// Socket inodes held by some process other than this one.
///
/// One walk of `/proc` answers for every keymap at once, which is why this hands back a
/// set rather than counting holders of a single socket. `None` means the walk itself
/// failed and nothing should be concluded from it; unreadable individual processes are
/// simply skipped, since a process we cannot look into is one we cannot serve either.
fn foreign_socket_inodes() -> Option<HashSet<u64>> {
	let own_pid = std::process::id().to_string();
	let mut inodes = HashSet::new();
	for process in std::fs::read_dir("/proc")
		.inspect_err(|err| tracing::error!("failed to read /proc: {err}"))
		.ok()?
		.flatten()
	{
		let pid = process.file_name();
		if pid == *own_pid || !pid.as_encoded_bytes()[0].is_ascii_digit() {
			continue;
		}
		let Ok(fds) = std::fs::read_dir(process.path().join("fd")) else {
			continue;
		};
		for fd in fds.flatten() {
			let Ok(target) = std::fs::read_link(fd.path()) else {
				continue;
			};
			if let Some(inode) = target
				.to_str()
				.and_then(|target| target.strip_prefix("socket:["))
				.and_then(|inode| inode.strip_suffix(']'))
				.and_then(|inode| inode.parse().ok())
			{
				inodes.insert(inode);
			}
		}
	}
	Some(inodes)
}

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
