mod core;
mod display;
mod dmabuf;
mod mesa_drm;
mod presentation;
mod registry;
mod relative_pointer;
mod util;
mod viewporter;
mod vulkano_data;
mod xdg;

use crate::core::error::ServerError;
use crate::core::registry::OwnedRegistry;
use crate::get_time;
use crate::nodes::drawable::model::ModelNodeSystemSet;
use crate::wayland::core::seat::SeatMessage;
use crate::wayland::core::surface::Surface;
use crate::wayland::presentation::MonotonicTimestamp;
use crate::wayland::util::ClientExt;
use crate::{BevyMaterial, core::task};
use bevy::app::{App, Plugin, Update};
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::{Local, Res, ResMut};
use bevy::prelude::{Deref, DerefMut};
use bevy::render::renderer::RenderDevice;
use bevy::render::{Render, RenderApp};
use bevy::{asset::Assets, ecs::resource::Resource, image::Image};
use bevy_dmabuf::import::ImportedDmatexs;
use bevy_mod_openxr::render::end_frame;
use bevy_mod_openxr::resources::{OxrFrameState, OxrInstance, Pipelined};
use bevy_mod_xr::session::XrRenderSet;
use core::buffer::BufferUsage;
use core::{buffer::Buffer, callback::Callback, surface::WL_SURFACE_REGISTRY};
use display::Display;
use mint::Vector2;
use pin_project_lite::pin_project;
use std::fs::File;
use std::io::ErrorKind;
use std::mem::MaybeUninit;
use std::time::Duration;
use std::{
	io,
	path::PathBuf,
	sync::{Arc, OnceLock},
};
use tokio::{net::UnixStream, sync::mpsc, task::AbortHandle};
use tokio_stream::{Stream, StreamExt};
use tracing::{debug_span, instrument};
use vulkano_data::setup_vulkano_context;
use waynest::{Connection, Socket};
use waynest::{ObjectId, ProtocolError};
use waynest_protocols::server::core::wayland::wl_display::WlDisplay;
use waynest_server::{Client as _, Listener, Store, StoreError};
use xdg::toplevel::Toplevel;

pub static WAYLAND_DISPLAY: OnceLock<PathBuf> = OnceLock::new();

#[derive(thiserror::Error, Debug)]
pub enum WaylandError {
	// #[error("Listener error: {0}")]
	// Listener(#[from] waynest_server::ListenerError),
	#[error("I/O error: {0}")]
	Io(#[from] io::Error),
	#[error("Decode error: {0}")]
	DecodeError(#[from] waynest::ProtocolError),
	#[error("Client requested unknown global: {0}")]
	UnknownGlobal(u32),
	#[error("No object found with ID {0}")]
	MissingObject(ObjectId),
	#[error("Fatal error on object {object_id} with code {code}: {message}")]
	Fatal {
		object_id: ObjectId,
		code: u32,
		message: &'static str,
	},
	#[error("Memfd error: {0}")]
	MemfdError(#[from] memfd::Error),
	#[error("Dmabuf import error: {0}")]
	DmabufImport(#[from] bevy_dmabuf::import::ImportError),
	#[error("Server error: {0}")]
	Server(#[from] ServerError),
	#[error("Failed to Insert Object")]
	FailedToInsertObject,
}
impl<T: Clone> From<StoreError<T>> for WaylandError {
	fn from(_value: StoreError<T>) -> Self {
		panic!("a");
		Self::FailedToInsertObject
	}
}

pin_project! {
	pub struct Client {
		store: Store<Client, WaylandError>,
		#[pin]
		connection: Socket,
		next_event_serial: u32,
	}
}
impl Connection for Client {
	type Error = WaylandError;

	fn fd(&mut self) -> Result<std::os::unix::prelude::OwnedFd, <Self as Connection>::Error> {
		Ok(self.connection.fd()?)
	}
}
impl Stream for Client {
	type Item = <Socket as Stream>::Item;

	fn poll_next(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Option<Self::Item>> {
		// <Socket as Stream>::poll_next(self.project().connection, cx)
		self.project().connection.poll_next(cx)
	}
}
impl futures_sink::Sink<waynest::Message> for Client {
	type Error = ProtocolError;

	fn poll_ready(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Result<(), Self::Error>> {
		self.project().connection.poll_ready(cx)
	}

	fn start_send(
		self: std::pin::Pin<&mut Self>,
		item: waynest::Message,
	) -> Result<(), Self::Error> {
		self.project().connection.start_send(item)
	}

	fn poll_flush(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Result<(), Self::Error>> {
		self.project().connection.poll_flush(cx)
	}

	fn poll_close(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Result<(), Self::Error>> {
		self.project().connection.poll_close(cx)
	}
}
impl Client {
	fn new(unix_stream: UnixStream) -> tokio::io::Result<Self> {
		Ok(Self {
			store: Store::new(),
			connection: Socket::new(unix_stream.into_std()?)?,
			next_event_serial: 0,
		})
	}
	pub fn next_event_serial(&mut self) -> u32 {
		let prev = self.next_event_serial;
		self.next_event_serial = self.next_event_serial.wrapping_add(1);
		prev
	}
}

impl waynest_server::Client for Client {
	type Store = Store<Client, WaylandError>;

	fn store(&self) -> &Self::Store {
		&self.store
	}

	fn store_mut(&mut self) -> &mut Self::Store {
		&mut self.store
	}
}

pub fn get_free_wayland_socket_path() -> Option<(PathBuf, File)> {
	// Use XDG runtime directory for secure, user-specific sockets
	let base_dirs = directories::BaseDirs::new()?;
	let runtime_dir = base_dirs.runtime_dir()?;

	// Iterate through conventional display numbers (matches X11 behavior)
	for display in 0..=32 {
		let socket_path = runtime_dir.join(format!("wayland-{display}"));
		let socket_lock_path = runtime_dir.join(format!("wayland-{display}.lock"));

		let Ok(lock) = File::create(&socket_lock_path) else {
			continue;
		};

		if lock.try_lock().is_err() {
			continue;
		};

		// Check for zombie sockets (file exists but nothing listening)
		if socket_path.exists() {
			match std::os::unix::net::UnixStream::connect(&socket_path) {
				Ok(_) => continue, // Active compositor found - skip
				Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
					// Stale socket - safe to remove since we hold the lock
					let _ = std::fs::remove_file(&socket_path);
				}
				Err(_) => continue, // Transient error - conservative skip
			}
		}

		// Found viable candidate: lock held, socket cleared/available
		return Some((socket_path, lock));
	}

	None // Exhausted all conventional display numbers
}

pub type WaylandResult<T, E = WaylandError> = std::result::Result<T, E>;

pub enum Message {
	Frame(Vec<Arc<Callback>>),
	ReleaseBuffer(Arc<Buffer>),
	CloseToplevel(Arc<Toplevel>),
	ResizeToplevel {
		toplevel: Arc<Toplevel>,
		size: Option<Vector2<u32>>,
	},
	ReconfigureToplevel(Arc<Toplevel>),
	SetToplevelVisualActive {
		toplevel: Arc<Toplevel>,
		active: bool,
	},
	Seat(SeatMessage),
	SendPresentationFeedback {
		surface: Arc<Surface>,
		display_timestamp: MonotonicTimestamp,
		refresh_cycle: u64,
	},
}

pub type MessageSink = mpsc::UnboundedSender<Message>;

#[derive(Debug)]
struct WaylandClient {
	abort_handle: AbortHandle,
}
impl WaylandClient {
	pub fn from_stream(socket: UnixStream) -> WaylandResult<Self> {
		let pid = socket.peer_cred().ok().and_then(|c| c.pid());
		let exe = pid.and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok());

		let mut client = Client::new(socket)?;
		let (message_sink, message_source) = mpsc::unbounded_channel();

		client.insert(ObjectId::DISPLAY, Display::new(message_sink, pid))?;

		let pid_printable = pid
			.map(|pid| pid.to_string())
			.unwrap_or_else(|| "??".to_string());
		let exe_printable = exe
			.and_then(|exe| {
				exe.file_name()
					.and_then(|exe| exe.to_str())
					.map(|exe| exe.to_string())
			})
			.unwrap_or_else(|| "??".to_string());
		let abort_handle = task::new(
			|| format!("Wayland client \"{exe_printable}\" dispatch, pid={pid_printable}"),
			Self::dispatch_loop(client, message_source),
		)?
		.abort_handle();

		Ok(WaylandClient { abort_handle })
	}

	async fn dispatch_loop(
		mut client: Client,
		mut render_message_rx: mpsc::UnboundedReceiver<Message>,
	) -> WaylandResult<()> {
		loop {
			tokio::select! {
				biased;
				// send all queued up messages
				msg = render_message_rx.recv() => {
					let Some(msg) = msg else {
						// Render message channel closed, end the dispatch loop
						return Ok(());
					};
					Self::handle_render_message(&mut client, msg).await?;
				}
				// handle the next message
				msg = client.try_next() => {
					let Some(mut msg) = msg? else {
						// Client disconnected, end the dispatch loop
						return Ok(());
					};
					if let Err(e) = client
						.get_raw(msg.object_id())
						.ok_or(WaylandError::MissingObject(msg.object_id()))?
						.dispatch_request(&mut client, msg.object_id(), &mut msg)
						.await
					{
						if let WaylandError::Fatal { object_id, code, message } = e {
							client.display().error(&mut client, ObjectId::DISPLAY, object_id, code, message.to_string()).await?;
						}
						tracing::error!("Wayland: {e}");
						return Err(e);
					}
				}
			};
		}
	}

	async fn handle_render_message(client: &mut Client, message: Message) -> WaylandResult<()> {
		use waynest_protocols::server::core::wayland::wl_buffer::WlBuffer;
		use waynest_protocols::server::core::wayland::wl_callback::WlCallback;
		use waynest_protocols::server::core::wayland::wl_display::WlDisplay;
		use waynest_protocols::server::stable::xdg_shell::xdg_toplevel::XdgToplevel;

		match message {
			Message::Frame(callbacks) => {
				let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
				let now = Duration::new(now.tv_sec as u64, now.tv_nsec as u32);
				let ms = (now.as_millis() % (u32::MAX as u128)) as u32;
				for callback in callbacks {
					callback.done(client, callback.0, ms).await?;
					client
						.get::<Display>(ObjectId::DISPLAY)
						.unwrap()
						.delete_id(client, ObjectId::DISPLAY, callback.0.as_raw())
						.await?;
					client.remove(callback.0);
				}
			}
			Message::ReleaseBuffer(buffer) => {
				buffer.release(client, buffer.id).await?;
			}
			Message::CloseToplevel(toplevel) => {
				toplevel.close(client, toplevel.id).await?;
			}
			Message::ResizeToplevel { toplevel, size } => {
				toplevel.set_size(size);
				toplevel.reconfigure(client).await?;
			}
			Message::ReconfigureToplevel(toplevel) => {
				toplevel.reconfigure(client).await?;
			}
			Message::SetToplevelVisualActive { toplevel, active } => {
				toplevel.set_activated(active);
				toplevel.reconfigure(client).await?;
			}
			Message::Seat(seat_message) => {
				if let Some(seat) = client.get::<Display>(ObjectId::DISPLAY).unwrap().seat.get() {
					seat.handle_message(client, seat_message).await?;
				}
			}
			Message::SendPresentationFeedback {
				surface,
				display_timestamp,
				refresh_cycle,
			} => {
				surface
					.send_presentation_feedback(client, display_timestamp, refresh_cycle)
					.await?;
			}
		}
		Ok(())
	}
}
impl Drop for WaylandClient {
	fn drop(&mut self) {
		self.abort_handle.abort();
	}
}

#[derive(Debug, Resource)]
pub struct Wayland {
	_lockfile: File,
	abort_handle: AbortHandle,
}
impl Wayland {
	pub fn new() -> color_eyre::eyre::Result<Self> {
		let (socket_path, _lockfile) = get_free_wayland_socket_path().ok_or(WaylandError::Io(
			std::io::ErrorKind::AddrNotAvailable.into(),
		))?;

		let _ = WAYLAND_DISPLAY.set(socket_path.clone());

		let listener = waynest_server::Listener::new_with_path(&socket_path)?;
		let _ = WAYLAND_DISPLAY.set(listener.socket_path().to_path_buf());

		let abort_handle = task::new(
			|| "Wayland socket accept loop",
			Self::handle_wayland_loop(listener),
		)?
		.abort_handle();

		Ok(Self {
			_lockfile,
			abort_handle,
		})
	}
	async fn handle_wayland_loop(mut listener: Listener) -> WaylandResult<()> {
		let mut clients = Vec::new();
		loop {
			if let Ok(Some(stream)) = listener.try_next().await {
				debug_span!("Accept wayland client").in_scope(|| {
					if let Ok(client) = WaylandClient::from_stream(stream) {
						clients.push(client);
					}
				});
			}
			clients.retain(|client| !client.abort_handle.is_finished());
		}

		#[allow(unreachable_code)]
		Ok(())
	}
}
impl Drop for Wayland {
	fn drop(&mut self) {
		self.abort_handle.abort();
	}
}

static RENDER_DEVICE: OnceLock<RenderDevice> = OnceLock::new();

pub struct WaylandPlugin;
impl Plugin for WaylandPlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Update, update_graphics.before(ModelNodeSystemSet));
		app.init_resource::<UsedBuffers>();
		app.sub_app_mut(RenderApp)
			.init_resource::<UsedBuffers>()
			.add_systems(
				Render,
				init_render_device.run_if(|| RENDER_DEVICE.get().is_none()),
			);
	}
	fn finish(&self, app: &mut App) {
		app.sub_app_mut(RenderApp)
			.add_systems(Render, setup_vulkano_context)
			.add_systems(Render, before_render.in_set(XrRenderSet::PreRender))
			.add_systems(Render, after_render.in_set(XrRenderSet::PostRender))
			.add_systems(
				Render,
				submit_frame_timings
					.in_set(XrRenderSet::PostRender)
					.after(end_frame),
			);
	}
}

fn init_render_device(dev: Res<RenderDevice>) {
	_ = RENDER_DEVICE.set(dev.clone());
}

#[derive(Resource, Deref, DerefMut)]
struct UsedBuffers(OwnedRegistry<BufferUsage>);
impl Default for UsedBuffers {
	fn default() -> Self {
		Self(OwnedRegistry::new())
	}
}

fn before_render(buffers: Res<UsedBuffers>) {
	for buf in WL_SURFACE_REGISTRY
		.get_valid_contents()
		.into_iter()
		.filter_map(|surface| surface.current_buffer_usage())
	{
		buffers.add_raw(buf);
	}
	for surface in WL_SURFACE_REGISTRY.get_valid_contents() {
		surface.frame_event();
	}
}

fn after_render(buffers: Res<UsedBuffers>) {
	buffers.clear();
}

#[instrument(level = "debug", name = "Wayland frame", skip_all)]
fn update_graphics(
	dmatexes: Res<ImportedDmatexs>,
	mut materials: ResMut<Assets<BevyMaterial>>,
	mut images: ResMut<Assets<Image>>,
) {
	for surface in WL_SURFACE_REGISTRY.get_valid_contents() {
		surface.update_graphics(&dmatexes, &mut materials, &mut images);
	}
}

#[instrument(level = "debug", name = "Wayland frame", skip_all)]
fn submit_frame_timings(
	mut frame_count: Local<u64>,
	instance: Option<Res<OxrInstance>>,
	frame_state: Option<Res<OxrFrameState>>,
	pipelined: Option<Res<Pipelined>>,
) {
	*frame_count += 1;
	let display_timestamp = frame_state
		.and_then(|state| Some((state, instance?)))
		.and_then(|(state, instance)| {
			instance
				.exts()
				.khr_convert_timespec_time
				.and_then(|v| unsafe {
					let mut out = MaybeUninit::uninit();
					let result = (v.convert_time_to_timespec_time)(
						instance.as_raw(),
						get_time(pipelined.is_some(), &state),
						out.as_mut_ptr(),
					);
					if result != openxr::sys::Result::SUCCESS {
						return None;
					}
					let v = out.assume_init();
					Some(rustix::time::Timespec {
						tv_sec: v.tv_sec,
						tv_nsec: v.tv_nsec,
					})
				})
		})
		.unwrap_or_else(|| rustix::time::clock_gettime(rustix::time::ClockId::Monotonic))
		.into();
	for surface in WL_SURFACE_REGISTRY.get_valid_contents() {
		surface.submit_presentation_feedback(display_timestamp, *frame_count);
	}
}
