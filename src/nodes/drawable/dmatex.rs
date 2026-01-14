use std::{
	os::fd::{AsFd as _, OwnedFd},
	sync::{Arc, LazyLock, OnceLock, Weak},
};

use bevy::{
	app::{Plugin, Update},
	asset::{Assets, Handle},
	ecs::{
		schedule::IntoScheduleConfigs as _,
		system::{Res, ResMut},
	},
	image::Image,
	render::{
		Render, RenderApp,
		camera::{ManualTextureView, ManualTextureViewHandle, ManualTextureViews},
		renderer::RenderDevice,
	},
};
use bevy_dmabuf::{
	dmatex::DmatexPlane,
	import::{ImportedDmatexs, ImportedTexture, import_texture},
};
use dashmap::DashMap;
use drm_fourcc::DrmFourcc;
use glam::UVec2;
use stardust_xr_server_foundation::{bail, error::Result};
use timeline_syncobj::{render_node::DrmRenderNode, timeline_syncobj::TimelineSyncObj};
use tracing::{error, warn};
use vulkano::sync::semaphore::{
	ExternalSemaphoreHandleType, ImportSemaphoreFdInfo, Semaphore, SemaphoreImportFlags,
};

use crate::{
	bevy_int::bevy_channel::{BevyChannel, BevyChannelReader},
	core::vulkano_data::VULKANO_CONTEXT,
	nodes::drawable::{DmatexSize, model::ModelNodeSystemSet},
};

#[derive(Debug)]
pub struct ImportedDmatex {
	tex: ImportedTexture,
	sync_obj: TimelineSyncObj,
	bevy_image_handle: OnceLock<Handle<bevy::image::Image>>,
	// TODO: handle destruction
	bevy_custom_view: OnceLock<ManualTextureViewHandle>,
}
pub static RENDER_DEV: OnceLock<RenderDevice> = OnceLock::new();
static DRM_RENDER_NODE: OnceLock<DrmRenderNode> = OnceLock::new();
static EXPORTED_DMATEXES: LazyLock<DashMap<u64, Weak<ImportedDmatex>>> =
	LazyLock::new(DashMap::new);
static NEW_DMATEXES: BevyChannel<Arc<ImportedDmatex>> = BevyChannel::new();
static DESTROYED_MANUAL_VIEWS: BevyChannel<ManualTextureViewHandle> = BevyChannel::new();
impl ImportedDmatex {
	pub fn import_uid(uid: u64) -> Option<Arc<Self>> {
		EXPORTED_DMATEXES.get(&uid)?.upgrade()
	}
	pub fn export_uid(self: &Arc<Self>) -> u64 {
		let id = rand::random();
		EXPORTED_DMATEXES.insert(id, Arc::downgrade(self));
		id
	}
	pub fn new(
		size: DmatexSize,
		format: u32,
		modifier: u64,
		srgb: bool,
		// TODO: impl
		array_layers: Option<u32>,
		planes: Vec<super::DmatexPlane>,
		timeline_syncobj_fd: OwnedFd,
	) -> Result<Arc<Self>> {
		let DmatexSize::Dim2D(res) = size else {
			bail!("non 2d dmatex are not implemented yet");
		};
		if array_layers.is_some_and(|v| v != 1) {
			bail!("array layers in dmatex is not implemented yet");
		}
		let vk = VULKANO_CONTEXT.wait();
		let render_node = match DRM_RENDER_NODE.get() {
			Some(v) => v,
			None => {
				let Some(render_node_id) = vk.get_drm_render_node_id() else {
					bail!("unable to get render_node");
				};
				let Ok(node) = DrmRenderNode::new(render_node_id & 0xFF)
					.inspect_err(|err| error!("unable to open render_node: {err}"))
				else {
					bail!("unable to open render_node");
				};
				_ = DRM_RENDER_NODE.set(node);
				DRM_RENDER_NODE.get().unwrap()
			}
		};
		let Ok(tex) = import_texture(
			RENDER_DEV.wait(),
			bevy_dmabuf::dmatex::Dmatex {
				planes: planes
					.into_iter()
					.map(|p| DmatexPlane {
						dmabuf_fd: p.dmabuf_fd.0.into(),
						modifier: modifier,
						offset: p.offset,
						stride: p.row_size as i32,
					})
					.collect(),
				res: bevy_dmabuf::dmatex::Resolution { x: res.x, y: res.y },
				format,
				flip_y: false,
				srgb,
			},
			bevy_dmabuf::import::DropCallback(None),
			bevy_dmabuf::import::DmatexUsage::Sampling,
		)
		.inspect_err(|err| error!("unable to import dmatex: {err}")) else {
			bail!("unable to import dmatex");
		};
		let Ok(sync_obj) = TimelineSyncObj::import(render_node, timeline_syncobj_fd.as_fd())
			.inspect_err(|err| error!("unable to import timiline syncobj: {err}"))
		else {
			bail!("unable to import timiline syncobj");
		};
		let tex = Arc::new(Self {
			tex,
			sync_obj,
			bevy_image_handle: OnceLock::new(),
			bevy_custom_view: OnceLock::new(),
		});
		NEW_DMATEXES.send(tex.clone());
		Ok(tex)
	}
	/// only use for readonly uses, write operations should sync with a vulkan semaphore
	pub fn signal_on_drop(self: &Arc<Self>, point: u64) -> SignalOnDrop {
		SignalOnDrop {
			point,
			tex: self.clone(),
			consumed: false,
		}
	}
	pub fn timeline_sync(&self) -> &TimelineSyncObj {
		&self.sync_obj
	}
	pub fn try_get_bevy_handle(&self) -> Option<Handle<bevy::image::Image>> {
		self.bevy_image_handle.get().cloned()
	}
	pub fn try_get_bevy_manual_view(&self) -> Option<ManualTextureViewHandle> {
		self.bevy_custom_view.get().cloned()
	}
	pub fn get_acquire_semaphore(&self, point: u64) -> Semaphore {
		let vk = VULKANO_CONTEXT.wait();
		let sema = Semaphore::from_pool(vk.dev.clone()).unwrap();
		let fd = self.timeline_sync().export_sync_file_point(point).unwrap();
		unsafe {
			sema.import_fd(ImportSemaphoreFdInfo {
				flags: SemaphoreImportFlags::TEMPORARY,
				file: Some(fd.into()),
				..ImportSemaphoreFdInfo::handle_type(ExternalSemaphoreHandleType::SyncFd)
			})
			.unwrap();
		}
		sema
	}
}
impl Drop for ImportedDmatex {
	fn drop(&mut self) {
		if let Some(view) = self.try_get_bevy_manual_view() {
			_ = DESTROYED_MANUAL_VIEWS.send(view);
		}
	}
}
#[derive(Debug)]
pub struct SignalOnDrop {
	point: u64,
	tex: Arc<ImportedDmatex>,
	consumed: bool,
}
impl SignalOnDrop {
	/// this semaphore has to already have a signal operation queued, or be signaled, and be
	/// created with the SyncFD export flag
	pub fn use_semaphore(mut self, semaphore: &Semaphore) {
		self.consumed = true;
		let fd = unsafe {
			semaphore
				.export_fd(ExternalSemaphoreHandleType::SyncFd)
				.unwrap()
		};
		self.tex
			.timeline_sync()
			.import_sync_file_point(fd.as_fd(), self.point)
			.unwrap();
	}
	pub fn tex_id(&self) -> u32 {
		self.tex.try_get_bevy_manual_view().unwrap().0
	}
}
impl Drop for SignalOnDrop {
	fn drop(&mut self) {
		if !self.consumed {
			unsafe {
				_ = self
					.tex
					.sync_obj
					.signal(self.point)
					.inspect_err(|err| warn!("failed to signal semaphore on drop: {err}"));
			}
		}
	}
}
pub struct DmatexPlugin;
impl Plugin for DmatexPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		NEW_DMATEXES.init(app);
		DESTROYED_MANUAL_VIEWS.init(app);
		app.add_systems(Update, add_dmatex_into_bevy.before(ModelNodeSystemSet));
		app.add_systems(Update, cleanup_manual_texture_views);
		app.sub_app_mut(RenderApp).add_systems(
			Render,
			init_render_device.run_if(|| RENDER_DEV.get().is_none()),
		);
	}
}
fn cleanup_manual_texture_views(
	mut destroyed_handles: ResMut<BevyChannelReader<ManualTextureViewHandle>>,
	mut custom_views: ResMut<ManualTextureViews>,
) {
	while let Some(e) = destroyed_handles.read() {
		custom_views.remove(&e);
	}
}
fn add_dmatex_into_bevy(
	mut images: ResMut<Assets<Image>>,
	texes: Res<ImportedDmatexs>,
	mut new_texes: ResMut<BevyChannelReader<Arc<ImportedDmatex>>>,
	mut custom_views: ResMut<ManualTextureViews>,
) {
	while let Some(tex) = new_texes.read() {
		if tex.bevy_image_handle.get().is_some() {
			continue;
		}
		let handle = ManualTextureViewHandle(rand::random());
		let wgpu_tex = tex.tex.texture();
		custom_views.insert(
			handle,
			ManualTextureView {
				texture_view: tex.tex.view(),
				size: UVec2::new(wgpu_tex.size().width, wgpu_tex.size().height),
				format: wgpu_tex.format(),
			},
		);
		_ = tex.bevy_custom_view.set(handle);
		let handle = texes.insert_imported_dmatex(&mut images, tex.tex.clone());
		_ = tex.bevy_image_handle.set(handle);
	}
}
fn init_render_device(dev: Res<RenderDevice>) {
	_ = RENDER_DEV.set(dev.clone());
}
pub const ALL_DRM_FOURCCS: [DrmFourcc; 105] = [
	DrmFourcc::Abgr1555,
	DrmFourcc::Abgr16161616f,
	DrmFourcc::Abgr2101010,
	DrmFourcc::Abgr4444,
	DrmFourcc::Abgr8888,
	DrmFourcc::Argb1555,
	DrmFourcc::Argb16161616f,
	DrmFourcc::Argb2101010,
	DrmFourcc::Argb4444,
	DrmFourcc::Argb8888,
	DrmFourcc::Axbxgxrx106106106106,
	DrmFourcc::Ayuv,
	DrmFourcc::Bgr233,
	DrmFourcc::Bgr565,
	DrmFourcc::Bgr565_a8,
	DrmFourcc::Bgr888,
	DrmFourcc::Bgr888_a8,
	DrmFourcc::Bgra1010102,
	DrmFourcc::Bgra4444,
	DrmFourcc::Bgra5551,
	DrmFourcc::Bgra8888,
	DrmFourcc::Bgrx1010102,
	DrmFourcc::Bgrx4444,
	DrmFourcc::Bgrx5551,
	DrmFourcc::Bgrx8888,
	DrmFourcc::Bgrx8888_a8,
	DrmFourcc::Big_endian,
	DrmFourcc::C8,
	DrmFourcc::Gr1616,
	DrmFourcc::Gr88,
	DrmFourcc::Nv12,
	DrmFourcc::Nv15,
	DrmFourcc::Nv16,
	DrmFourcc::Nv21,
	DrmFourcc::Nv24,
	DrmFourcc::Nv42,
	DrmFourcc::Nv61,
	DrmFourcc::P010,
	DrmFourcc::P012,
	DrmFourcc::P016,
	DrmFourcc::P210,
	DrmFourcc::Q401,
	DrmFourcc::Q410,
	DrmFourcc::R16,
	DrmFourcc::R8,
	DrmFourcc::Rg1616,
	DrmFourcc::Rg88,
	DrmFourcc::Rgb332,
	DrmFourcc::Rgb565,
	DrmFourcc::Rgb565_a8,
	DrmFourcc::Rgb888,
	DrmFourcc::Rgb888_a8,
	DrmFourcc::Rgba1010102,
	DrmFourcc::Rgba4444,
	DrmFourcc::Rgba5551,
	DrmFourcc::Rgba8888,
	DrmFourcc::Rgbx1010102,
	DrmFourcc::Rgbx4444,
	DrmFourcc::Rgbx5551,
	DrmFourcc::Rgbx8888,
	DrmFourcc::Rgbx8888_a8,
	DrmFourcc::Uyvy,
	DrmFourcc::Vuy101010,
	DrmFourcc::Vuy888,
	DrmFourcc::Vyuy,
	DrmFourcc::X0l0,
	DrmFourcc::X0l2,
	DrmFourcc::Xbgr1555,
	DrmFourcc::Xbgr16161616f,
	DrmFourcc::Xbgr2101010,
	DrmFourcc::Xbgr4444,
	DrmFourcc::Xbgr8888,
	DrmFourcc::Xbgr8888_a8,
	DrmFourcc::Xrgb1555,
	DrmFourcc::Xrgb16161616f,
	DrmFourcc::Xrgb2101010,
	DrmFourcc::Xrgb4444,
	DrmFourcc::Xrgb8888,
	DrmFourcc::Xrgb8888_a8,
	DrmFourcc::Xvyu12_16161616,
	DrmFourcc::Xvyu16161616,
	DrmFourcc::Xvyu2101010,
	DrmFourcc::Xyuv8888,
	DrmFourcc::Y0l0,
	DrmFourcc::Y0l2,
	DrmFourcc::Y210,
	DrmFourcc::Y212,
	DrmFourcc::Y216,
	DrmFourcc::Y410,
	DrmFourcc::Y412,
	DrmFourcc::Y416,
	DrmFourcc::Yuv410,
	DrmFourcc::Yuv411,
	DrmFourcc::Yuv420,
	DrmFourcc::Yuv420_10bit,
	DrmFourcc::Yuv420_8bit,
	DrmFourcc::Yuv422,
	DrmFourcc::Yuv444,
	DrmFourcc::Yuyv,
	DrmFourcc::Yvu410,
	DrmFourcc::Yvu411,
	DrmFourcc::Yvu420,
	DrmFourcc::Yvu422,
	DrmFourcc::Yvu444,
	DrmFourcc::Yvyu,
];
