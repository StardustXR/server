use stereokit_rust::system::World;
use zbus::{Connection, ObjectServer, interface};

pub struct PlaySpaceBounds;
impl PlaySpaceBounds {
	pub async fn create(connection: &Connection) {
		connection
			.object_server()
			.at("/org/stardustxr/PlaySpace", Self)
			.await
			.unwrap();
	}
}
#[interface(name = "org.stardustxr.PlaySpace")]
impl PlaySpaceBounds {
	#[zbus(property)]
	fn bounds(&self) -> Vec<(f64, f64)> {
		let bounds = World::get_bounds_size();
		vec![
			((bounds.x).into(), (bounds.y).into()),
			((bounds.x).into(), (-bounds.y).into()),
			((-bounds.x).into(), (-bounds.y).into()),
			((-bounds.x).into(), (bounds.y).into()),
		]
	}
}
