use stardust_xr::values::color::{AlphaColor, Rgb, color_space::LinearRgb};

pub trait ColorConvert {
	fn to_bevy(&self) -> bevy::color::Color;
}
// even tho its supposed to be linear the values have to be interpreted as Srgba to produce the
// correct result because StereoKit used Srgba while it was assumed that is uses linear rgba
impl ColorConvert for AlphaColor<f32, Rgb<f32, LinearRgb>> {
	fn to_bevy(&self) -> bevy::color::Color {
		bevy::color::Color::srgba(self.c.r, self.c.g, self.c.b, self.a)
	}
}
