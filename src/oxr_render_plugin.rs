use bevy::{
	app::{App, Plugin, PostUpdate},
	prelude::{resource_added, IntoSystemConfigs as _, TransformSystem},
	render::{
		extract_resource::ExtractResourcePlugin, pipelined_rendering::PipelinedRenderingPlugin,
		Render, RenderApp,
	},
};
use bevy_mod_openxr::{
	init::should_run_frame_loop,
	layer_builder::ProjectionLayer,
	render::{
		begin_frame, clean_views, end_frame, init_views, insert_texture_views, locate_views,
		release_image, update_views, update_views_render_world, wait_image,
	},
	resources::{
		OxrFrameState, OxrGraphicsInfo, OxrRenderLayers, OxrSwapchainImages, OxrViews, Pipelined,
	},
	session::OxrSession,
};
use bevy_mod_xr::session::{XrPreDestroySession, XrRenderSet, XrSessionCreated};

pub struct StardustOxrRenderPlugin;

impl Plugin for StardustOxrRenderPlugin {
	fn build(&self, app: &mut App) {
		if app.is_plugin_added::<PipelinedRenderingPlugin>() {
			app.init_resource::<Pipelined>();
		}

		app.add_plugins((
			ExtractResourcePlugin::<OxrFrameState>::default(),
			ExtractResourcePlugin::<OxrGraphicsInfo>::default(),
			ExtractResourcePlugin::<OxrSwapchainImages>::default(),
			ExtractResourcePlugin::<OxrViews>::default(),
		))
		.add_systems(XrPreDestroySession, clean_views)
		.add_systems(
			XrSessionCreated,
			init_views.run_if(resource_added::<OxrSession>),
		)
		.add_systems(
			PostUpdate,
			(locate_views, update_views)
				.before(TransformSystem::TransformPropagate)
				.chain()
				.run_if(should_run_frame_loop),
		)
		.init_resource::<OxrViews>();

		let render_app = app.sub_app_mut(RenderApp);

		render_app
			.add_systems(XrPreDestroySession, clean_views)
			.add_systems(
				Render,
				(
					begin_frame,
					insert_texture_views,
					locate_views,
					update_views_render_world,
					wait_image,
				)
					.chain()
					.in_set(XrRenderSet::PreRender)
					.run_if(should_run_frame_loop),
			)
			.add_systems(
				Render,
				(release_image, end_frame)
					.chain()
					.run_if(should_run_frame_loop)
					.in_set(XrRenderSet::PostRender),
			)
			.insert_resource(OxrRenderLayers(vec![Box::new(ProjectionLayer)]));
	}
}
