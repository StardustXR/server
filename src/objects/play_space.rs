use zbus::{interface, Connection, ObjectServer};

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
	// TODO: reimplement under bevy
	#[zbus(property)]
	fn bounds(&self) -> Vec<(f64, f64)> {
		// let bounds = World::get_bounds_size();
		// vec![
		// 	((bounds.x).into(), (bounds.y).into()),
		// 	((bounds.x).into(), (-bounds.y).into()),
		// 	((-bounds.x).into(), (-bounds.y).into()),
		// 	((-bounds.x).into(), (bounds.y).into()),
		// ]
		vec![]
	}
}
