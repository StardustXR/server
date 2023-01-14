mod core;
mod nodes;
mod objects;
#[cfg(feature = "wayland")]
mod wayland;

use crate::core::destroy_queue;
use crate::nodes::{drawable, hmd, input};
use crate::objects::input::mouse_pointer::MousePointer;
use crate::objects::input::sk_controller::SkController;
use crate::objects::input::sk_hand::SkHand;

use self::core::eventloop::EventLoop;
use clap::Parser;
use color_eyre::eyre::Result;
use directories::ProjectDirs;
use std::sync::Arc;
use stereokit::input::Handed;
use stereokit::lifecycle::DepthMode;
use stereokit::lifecycle::DisplayMode;
use stereokit::render::SphericalHarmonics;
use stereokit::render::StereoKitRender;
use stereokit::texture::Texture;
use stereokit::time::StereoKitTime;
use tokio::{runtime::Handle, sync::oneshot};
use tracing::info;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, long, action)]
	flatscreen: bool,

	/// Run Stardust XR as an overlay with given priority
	#[clap(id = "PRIORITY", short = 'o', long = "overlay", action)]
	overlay_priority: Option<u32>,

	/// Don't create a tip input for controller because SOME RUNTIMES will lie
	#[clap(long, action)]
	disable_controller: bool,
}

fn main() -> Result<()> {
	#[cfg(not(feature = "profile"))]
	tracing_subscriber::fmt::init();
	#[cfg(feature = "profile")]
	console_subscriber::init();
	let project_dirs = ProjectDirs::from("", "", "stardust").unwrap();
	let cli_args = Arc::new(CliArgs::parse());

	let stereokit = stereokit::Settings::default()
		.app_name("Stardust XR")
		.overlay_app(cli_args.overlay_priority.is_some())
		.overlay_priority(cli_args.overlay_priority.unwrap_or(u32::MAX))
		.disable_desktop_input_window(true)
		.display_preference(if cli_args.flatscreen {
			DisplayMode::Flatscreen
		} else {
			DisplayMode::MixedReality
		})
		.depth_mode(DepthMode::D32)
		.init()
		.expect("StereoKit failed to initialize");
	info!("Init StereoKit");

	// Skytex/light stuff
	{
		let skytex_path = project_dirs.config_dir().join("skytex.hdr");
		if let Some((tex, light)) = skytex_path
			.exists()
			.then(|| Texture::from_cubemap_equirectangular(&stereokit, &skytex_path, true, 100))
			.flatten()
		{
			stereokit.set_skytex(&tex);
			stereokit.set_skylight(&light);
		} else if let Some(tex) = Texture::cubemap_from_spherical_harmonics(
			&stereokit,
			&SphericalHarmonics::default(),
			16,
			0.0,
			0.0,
		) {
			stereokit.set_skytex(&tex);
		}
	}

	let mouse_pointer = cli_args.flatscreen.then(MousePointer::new);
	let mut hands =
		(!cli_args.flatscreen).then(|| [SkHand::new(Handed::Left), SkHand::new(Handed::Right)]);
	let mut controllers = (!cli_args.flatscreen && !cli_args.disable_controller).then(|| {
		[
			SkController::new(Handed::Left),
			SkController::new(Handed::Right),
		]
	});

	if hands.is_none() {
		unsafe {
			stereokit::sys::input_hand_visible(stereokit::sys::handed__handed_left, false as i32);
			stereokit::sys::input_hand_visible(stereokit::sys::handed__handed_right, false as i32);
		}
	}

	let (event_stop_tx, event_stop_rx) = oneshot::channel::<()>();
	let (handle_sender, handle_receiver) = oneshot::channel::<Handle>();
	let event_thread = std::thread::Builder::new()
		.name("event_loop".to_owned())
		.spawn(move || event_loop(handle_sender, event_stop_rx))?;
	let _tokio_handle = handle_receiver.blocking_recv()?.enter();

	#[cfg(feature = "wayland")]
	let mut wayland = wayland::Wayland::new()?;
	info!("Stardust ready!");
	stereokit.run(
		|sk| {
			hmd::frame(sk);
			#[cfg(feature = "wayland")]
			wayland.frame(sk);
			destroy_queue::clear();

			if let Some(mouse_pointer) = &mouse_pointer {
				mouse_pointer.update(sk);
			}
			if let Some(hands) = &mut hands {
				hands[0].update(sk);
				hands[1].update(sk);
			}
			if let Some(controllers) = &mut controllers {
				controllers[0].update(sk);
				controllers[1].update(sk);
			}
			input::process_input();
			nodes::root::Root::logic_step(sk.time_elapsed());
			drawable::draw(sk);

			#[cfg(feature = "wayland")]
			wayland.make_context_current();
		},
		|_| {
			info!("Cleanly shut down StereoKit");
		},
	);

	#[cfg(feature = "wayland")]
	drop(wayland);

	let _ = event_stop_tx.send(());
	event_thread
		.join()
		.expect("Failed to cleanly shut down event loop")?;
	info!("Cleanly shut down Stardust");
	Ok(())
}

// #[tokio::main]
#[tokio::main(flavor = "current_thread")]
async fn event_loop(
	handle_sender: oneshot::Sender<Handle>,
	stop_rx: oneshot::Receiver<()>,
) -> color_eyre::eyre::Result<()> {
	let _ = handle_sender.send(Handle::current());
	// console_subscriber::init();

	let event_loop = EventLoop::new().expect("Couldn't create server socket");
	info!("Init event loop");
	info!(
		"Stardust socket created at {}",
		event_loop.socket_path.display()
	);

	let result = tokio::select! {
		biased;
		_ = tokio::signal::ctrl_c() => Ok(()),
		_ = stop_rx => Ok(()),
	};

	info!("Cleanly shut down event loop");

	unsafe {
		stereokit::sys::sk_quit();
	}

	result
}
