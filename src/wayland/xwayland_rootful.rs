use crate::core::{client::get_env, task};
use crate::STOP_NOTIFIER;
use smithay::reexports::rustix;
use smithay::reexports::rustix::io::{fcntl_setfd, Errno, FdFlags};
use smithay::reexports::rustix::net::SocketAddrUnix;
use std::io::{Read, Write};
use std::{
	io::ErrorKind,
	os::{
		fd::{AsRawFd, BorrowedFd, RawFd},
		unix::process::CommandExt,
	},
	process::{ChildStdout, Command, Stdio},
};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use super::X_DISPLAY;

pub fn start_xwayland(wayland_socket: RawFd) -> std::io::Result<X11Lock> {
	let (mut lock, listener) = bind_socket()?;

	let abort_handle = task::new(|| "X11 Client Acceptor", async move {
		loop {
			let Ok((stream, _)) = tokio::select! {
				_ = STOP_NOTIFIER.notified() => break,
				e = listener.accept() => e,
			} else {
				continue;
			};

			let Ok((x_wm_x11, _x_wm_me)) = UnixStream::pair() else {
				continue;
			};
			let Ok(env) = stream
				.peer_cred()
				.and_then(|c| c.pid().ok_or(ErrorKind::Other.into()))
				.and_then(get_env)
			else {
				continue;
			};

			let _ = spawn_xwayland(
				lock.display,
				wayland_socket,
				x_wm_x11,
				stream.as_raw_fd(),
				env.get("STARDUST_STARTUP_TOKEN").cloned(),
			);
		}
	})
	.map_err(|_| ErrorKind::Other)?
	.abort_handle();
	lock.x_abort_handle.replace(abort_handle);
	let _ = X_DISPLAY.set(lock.display);
	Ok(lock)
}

/// Find a free X11 display slot and setup
pub(crate) fn bind_socket() -> Result<(X11Lock, UnixListener), std::io::Error> {
	for d in 0..33 {
		// if fails, try the next one
		if let Ok(lock) = X11Lock::grab(d) {
			// we got a lockfile, try and create the socket
			match open_x11_socket_for_display(d) {
				Ok(socket) => return Ok((lock, socket)),
				Err(err) => warn!(display = d, "Failed to create sockets: {}", err),
			}
		}
	}
	// If we reach here, all values from 0 to 32 failed
	// we need to stop trying at some point

	Err(std::io::Error::new(
		std::io::ErrorKind::AddrInUse,
		"Could not find a free socket for the XServer.",
	))
}

#[derive(Debug)]
pub(crate) struct X11Lock {
	display: u32,
	x_abort_handle: Option<AbortHandle>,
}

impl X11Lock {
	/// Try to grab a lockfile for given X display number
	fn grab(number: u32) -> Result<X11Lock, ()> {
		debug!(display = number, "Attempting to aquire an X11 display lock");
		let filename = format!("/tmp/.X{}-lock", number);
		let lockfile = ::std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&filename);
		match lockfile {
			Ok(mut file) => {
				// we got it, write our PID in it and we're good
				let ret = file.write_fmt(format_args!(
					"{:>10}\n",
					rustix::process::Pid::as_raw(Some(rustix::process::getpid()))
				));
				if ret.is_err() {
					// write to the file failed ? we abandon
					::std::mem::drop(file);
					let _ = ::std::fs::remove_file(&filename);
					Err(())
				} else {
					debug!(display = number, "X11 lock acquired");
					// we got the lockfile and wrote our pid to it, all is good
					Ok(X11Lock {
						display: number,
						x_abort_handle: None,
					})
				}
			}
			Err(_) => {
				debug!(display = number, "Failed to acquire lock");
				// we could not open the file, now we try to read it
				// and if it contains the pid of a process that no longer
				// exist (so if a previous x server claimed it and did not
				// exit gracefully and remove it), we claim it
				// if we can't open it, give up
				let mut file = ::std::fs::File::open(&filename).map_err(|_| ())?;
				let mut spid = [0u8; 11];
				file.read_exact(&mut spid).map_err(|_| ())?;
				::std::mem::drop(file);
				let pid = rustix::process::Pid::from_raw(
					::std::str::from_utf8(&spid)
						.map_err(|_| ())?
						.trim()
						.parse::<i32>()
						.map_err(|_| ())?,
				)
				.ok_or(())?;
				if let Err(Errno::SRCH) = rustix::process::test_kill_process(pid) {
					// no process whose pid equals the contents of the lockfile exists
					// remove the lockfile and try grabbing it again
					if let Ok(()) = ::std::fs::remove_file(filename) {
						debug!(
							display = number,
							"Lock was blocked by a defunct X11 server, trying again"
						);
						return X11Lock::grab(number);
					} else {
						// we could not remove the lockfile, abort
						return Err(());
					}
				}
				// if we reach here, this lockfile exists and is probably in use, give up
				Err(())
			}
		}
	}

	pub(crate) fn display(&self) -> u32 {
		self.display
	}
}

impl Drop for X11Lock {
	fn drop(&mut self) {
		info!("Cleaning up X11 lock.");
		// Cleanup all the X11 files
		if let Err(e) = ::std::fs::remove_file(format!("/tmp/.X11-unix/X{}", self.display)) {
			warn!(error = ?e, "Failed to remove X11 socket");
		}
		if let Err(e) = ::std::fs::remove_file(format!("/tmp/.X{}-lock", self.display)) {
			warn!(error = ?e, "Failed to remove X11 lockfile");
		}
		if let Some(join_handle) = self.x_abort_handle.take() {
			join_handle.abort();
		}
	}
}

/// Open the two unix sockets an X server listens on
///
/// Should only be done after the associated lockfile is acquired!
fn open_x11_socket_for_display(display: u32) -> rustix::io::Result<UnixListener> {
	let path = format!("/tmp/.X11-unix/X{}", display);
	let _ = ::std::fs::remove_file(&path);
	// We know this path is not too long, these unwrap cannot fail
	let fs_addr = SocketAddrUnix::new(path.as_bytes()).unwrap();
	open_socket(fs_addr)
}

/// Open an unix socket for listening and bind it to given path
fn open_socket(addr: SocketAddrUnix) -> rustix::io::Result<UnixListener> {
	// create an unix stream socket
	let fd = rustix::net::socket_with(
		rustix::net::AddressFamily::UNIX,
		rustix::net::SocketType::STREAM,
		rustix::net::SocketFlags::CLOEXEC,
		None,
	)?;
	// bind it to requested address
	rustix::net::bind_unix(&fd, &addr)?;
	rustix::net::listen(&fd, 1)?;
	Ok(UnixListener::from_std(std::os::unix::net::UnixListener::from(fd)).unwrap())
}

fn spawn_xwayland(
	display: u32,
	wayland_socket: RawFd,
	wm_socket: UnixStream,
	listen_socket: RawFd,
	stardust_startup_token: Option<String>,
) -> std::io::Result<ChildStdout> {
	let mut command = Command::new("sh");

	// We use output stream to communicate because FD is easier to handle than exit code.
	command.stdout(Stdio::piped());

	let mut xwayland_args = format!(":{} -geometry 1920x1080", display);
	xwayland_args.push_str(&format!(" -listenfd {}", listen_socket));

	// This command let sh to:
	// * Set up signal handler for USR1
	// * Launch Xwayland with USR1 ignored so Xwayland will signal us when it is ready (also redirect
	//   Xwayland's STDOUT to STDERR so its output, if any, won't distract us)
	// * Print "S" and exit if USR1 is received
	command.arg("-c").arg(format!(
		"trap 'echo S' USR1; (trap '' USR1; exec Xwayland {}) 1>&2 & wait",
		xwayland_args
	));

	// Setup the environment: clear everything except PATH and XDG_RUNTIME_DIR
	command.env_clear();
	for (key, value) in std::env::vars_os() {
		if key.to_str() == Some("PATH") || key.to_str() == Some("XDG_RUNTIME_DIR") {
			command.env(key, value);
			continue;
		}
	}
	command.env("WAYLAND_SOCKET", format!("{}", wayland_socket.as_raw_fd()));
	command.env(
		"STARDUST_STARTUP_TOKEN",
		stardust_startup_token.unwrap_or_default(),
	);

	unsafe {
		let wayland_socket_fd = wayland_socket.as_raw_fd();
		let wm_socket_fd = wm_socket.as_raw_fd();
		command.pre_exec(move || {
			// unset the CLOEXEC flag from the sockets we need to pass
			// to xwayland
			unset_cloexec(wayland_socket_fd)?;
			unset_cloexec(wm_socket_fd)?;
			unset_cloexec(listen_socket)?;
			Ok(())
		});
	}

	let mut child = command.spawn()?;
	Ok(child.stdout.take().expect("stdout should be piped"))
}

/// Remove the `O_CLOEXEC` flag from this `Fd`
///
/// This means that the `Fd` will *not* be automatically
/// closed when we `exec()` into XWayland
unsafe fn unset_cloexec(fd: RawFd) -> std::io::Result<()> {
	let fd = BorrowedFd::borrow_raw(fd);
	fcntl_setfd(fd, FdFlags::empty())?;
	Ok(())
}
