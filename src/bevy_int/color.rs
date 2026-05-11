pub trait ColorConvert {
	fn to_bevy(&self) -> bevy::color::Color;
}
impl ColorConvert for stardust_xr_protocol::types::Color {
	fn to_bevy(&self) -> bevy::color::Color {
		bevy::color::Color::linear_rgba(self.r, self.g, self.b, self.a)
	}
}
