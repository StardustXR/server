use color_eyre::eyre::Result;
use smithay::{reexports::wayland_server::DisplayHandle, xwayland::XWayland};
use std::{process::Stdio, time::Duration};
use tokio::io::unix::AsyncFd;

use super::X_DISPLAY;

pub async fn start_xwayland(dh: DisplayHandle) -> Result<()> {
	let (mut xwayland, client) = XWayland::spawn(
		&dh,
		None,
		std::iter::empty::<(String, String)>(),
		true,
		Stdio::null(),
		Stdio::null(),
		|_| (),
	)?;

	// just wait until it's readable
	drop(AsyncFd::new(xwayland.poll_fd())?.readable().await?);
	let wm_socket = xwayland.take_socket()?.unwrap();

	let _ = X_DISPLAY.set(xwayland.display_number());

	println!("yippeee x is available at :{}", xwayland.display_number());

	tokio::time::sleep(Duration::from_secs(100000000000)).await;
	Ok(())
}
