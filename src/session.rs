use crate::core::client::CLIENTS;
use crate::core::client_state::ClientStateParsed;
#[cfg(feature = "wayland")]
use crate::wayland::WAYLAND_DISPLAY;
use crate::{CliArgs, STARDUST_INSTANCE};
use directories::ProjectDirs;
use rustc_hash::FxHashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::task::LocalSet;
use tracing::info;

pub async fn save_session(project_dirs: &ProjectDirs) {
	let session_id = nanoid::nanoid!();
	let state_dir = project_dirs.state_dir().unwrap();
	let session_dir = state_dir.join(&session_id);
	std::fs::create_dir_all(&session_dir).unwrap();
	let _ = std::fs::remove_dir_all(state_dir.join("latest"));
	std::os::unix::fs::symlink(&session_dir, state_dir.join("latest")).unwrap();

	let local_set = LocalSet::new();
	for client in CLIENTS.get_vec() {
		let session_dir = session_dir.clone();
		local_set.spawn_local(async move {
			tokio::select! {
				biased;
				s = client.save_state() => {if let Some(s) = s { s.to_file(&session_dir) }},
				_ = tokio::time::sleep(Duration::from_millis(100)) => (),
			}
		});
	}
	local_set.await;
	info!("Session ID for restore is {session_id}");
}

pub fn launch_start(cli_args: &CliArgs, project_dirs: &ProjectDirs) -> Vec<Child> {
	match (&cli_args.restore, &cli_args.startup_script) {
		(Some(session_id), _) => restore_session(
			&project_dirs.state_dir().unwrap().join(session_id),
			cli_args.debug_launched_clients,
		),
		(None, Some(startup_script)) => run_script(
			&startup_script.clone().canonicalize().unwrap_or_default(),
			cli_args.debug_launched_clients,
		),
		(None, None) => run_script(
			&project_dirs.config_dir().join("startup"),
			cli_args.debug_launched_clients,
		),
	}
}

pub fn restore_session(session_dir: &Path, debug_launched_clients: bool) -> Vec<Child> {
	let Ok(clients) = session_dir.read_dir() else {
		return Vec::new();
	};
	clients
		.filter_map(Result::ok)
		.filter_map(|c| ClientStateParsed::from_file(&c.path()))
		.filter_map(ClientStateParsed::launch_command)
		.filter_map(|c| run_client(c, debug_launched_clients))
		.collect()
}

pub fn run_script(script_path: &Path, debug_launched_clients: bool) -> Vec<Child> {
	let _ = std::fs::set_permissions(script_path, std::fs::Permissions::from_mode(0o755));
	let startup_command = Command::new(script_path);
	run_client(startup_command, debug_launched_clients)
		.map(|c| vec![c])
		.unwrap_or_default()
}

pub fn run_client(mut command: Command, debug_launched_clients: bool) -> Option<Child> {
	command.stdin(Stdio::null());
	if !debug_launched_clients {
		command.stdout(Stdio::null());
		command.stderr(Stdio::null());
	}
	for (var, value) in connection_env() {
		command.env(var, value);
	}
	let child = command.spawn().ok()?;
	Some(child)
}

pub fn connection_env() -> FxHashMap<String, String> {
	macro_rules! var_env_insert {
		($env:ident, $name:ident) => {
			$env.insert(stringify!($name).to_string(), $name.get().unwrap().clone());
		};
	}

	let mut env: FxHashMap<String, String> = FxHashMap::default();
	var_env_insert!(env, STARDUST_INSTANCE);

	if let Some(flat_wayland_display) = std::env::var_os("WAYLAND_DISPLAY") {
		env.insert(
			"FLAT_WAYLAND_DISPLAY".to_string(),
			flat_wayland_display.to_string_lossy().into_owned(),
		);
	}
	#[cfg(feature = "wayland")]
	{
		var_env_insert!(env, WAYLAND_DISPLAY);
		env.insert("XDG_SESSION_TYPE".to_string(), "wayland".to_string());
		env.insert("GDK_BACKEND".to_string(), "wayland".to_string());
		env.insert("QT_QPA_PLATFORM".to_string(), "wayland".to_string());
		env.insert("MOZ_ENABLE_WAYLAND".to_string(), "1".to_string());
		env.insert("CLUTTER_BACKEND".to_string(), "wayland".to_string());
		env.insert("SDL_VIDEODRIVER".to_string(), "wayland".to_string());
	}
	env
}
