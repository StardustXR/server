#[rustfmt::skip]
#[path = "../../src/debug_protocol.rs"]
mod debug_protocol;

use debug_protocol::ServerDebug;
use gluon_ipc::Ref;
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
	let Some(path) = stardust_xr_protocol::dir::find_ref_file("stardust-debug") else {
		eprintln!("no running stardust server found");
		return ExitCode::FAILURE;
	};
	let debug = match Ref::connect(path).await {
		Ok(r) => ServerDebug::from_ref(r),
		Err(e) => {
			eprintln!("couldn't connect to the server: {e}");
			return ExitCode::FAILURE;
		}
	};
	match debug.toggle_field_gizmos().await {
		Ok(enabled) => {
			println!("field gizmos {}", if enabled { "on" } else { "off" });
			ExitCode::SUCCESS
		}
		Err(e) => {
			eprintln!("toggle failed: {e}");
			ExitCode::FAILURE
		}
	}
}
