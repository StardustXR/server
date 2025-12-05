use stardust_xr_wire::values::color::{AlphaColor, Rgb, color_space::LinearRgb};

pub trait ColorConvert {
	fn to_bevy(&self) -> bevy::color::Color;
}
impl ColorConvert for AlphaColor<f32, Rgb<f32, LinearRgb>> {
	fn to_bevy(&self) -> bevy::color::Color {
		bevy::color::Color::linear_rgba(self.c.r, self.c.g, self.c.b, self.a)
	}
}
