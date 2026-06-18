//! Shared rendering helpers for infobar images.
//!
//! Every action draws onto the same 248x58 infobar canvas, so the canvas
//! creation, font loading and PNG/base64 encoding live here instead of being
//! copy-pasted into each action.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use base64::{Engine as _, engine::general_purpose};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use std::io::Cursor;
use std::sync::OnceLock;

/// Infobar surface dimensions in pixels.
pub const WIDTH: u32 = 248;
pub const HEIGHT: u32 = 58;

pub const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
pub const DIM: Rgba<u8> = Rgba([150, 150, 150, 255]);
pub const TRACK: Rgba<u8> = Rgba([55, 55, 60, 255]);

/// The bundled font, parsed once and reused for every draw call.
fn font() -> &'static FontRef<'static> {
	static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
	FONT.get_or_init(|| FontRef::try_from_slice(include_bytes!("../assets/fonts/Roboto-Regular.ttf")).expect("bundled Roboto font is valid"))
}

/// A black infobar canvas ready to be drawn onto.
pub struct Canvas {
	image: RgbaImage,
}

impl Canvas {
	/// Create a fresh black canvas at the infobar resolution.
	pub fn new() -> Self {
		Self::with_size(WIDTH, HEIGHT)
	}

	/// Create a fresh black canvas at an arbitrary size (e.g. a square keypad key).
	pub fn with_size(width: u32, height: u32) -> Self {
		let mut image = RgbaImage::new(width, height);
		for pixel in image.pixels_mut() {
			*pixel = Rgba([0, 0, 0, 255]);
		}
		Self { image }
	}

	/// Draw left-aligned text at `(x, y)` with the given pixel size and color.
	pub fn text(&mut self, x: i32, y: i32, size: f32, color: Rgba<u8>, text: &str) -> &mut Self {
		let scale = PxScale { x: size, y: size };
		draw_text_mut(&mut self.image, color, x, y, scale, font(), text);
		self
	}

	/// Draw text horizontally centered on `cx`.
	pub fn text_centered(&mut self, cx: i32, y: i32, size: f32, color: Rgba<u8>, text: &str) -> &mut Self {
		let w = text_width(text, size);
		self.text(cx - w / 2, y, size, color, text)
	}

	/// Draw a circular progress gauge (donut): a dim track ring with a colored
	/// arc that fills clockwise from the top (12 o'clock) proportional to
	/// `fraction` (clamped to `0.0..=1.0`). 4x4 supersampling smooths the edges.
	pub fn gauge(&mut self, cx: f32, cy: f32, r_outer: f32, r_inner: f32, fraction: f32, fill: Rgba<u8>) -> &mut Self {
		use std::f32::consts::TAU;
		const SS: i32 = 4;
		let thresh = fraction.clamp(0.0, 1.0) * TAU;
		let n = (SS * SS) as f32;

		let x0 = (cx - r_outer).floor() as i32;
		let x1 = (cx + r_outer).ceil() as i32;
		let y0 = (cy - r_outer).floor() as i32;
		let y1 = (cy + r_outer).ceil() as i32;

		let (w, h) = (self.image.width(), self.image.height());
		for py in y0..=y1 {
			for px in x0..=x1 {
				if px < 0 || py < 0 || px as u32 >= w || py as u32 >= h {
					continue;
				}
				let bg = *self.image.get_pixel(px as u32, py as u32);
				let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
				for sy in 0..SS {
					for sx in 0..SS {
						let fx = px as f32 + (sx as f32 + 0.5) / SS as f32;
						let fy = py as f32 + (sy as f32 + 0.5) / SS as f32;
						let (dx, dy) = (fx - cx, fy - cy);
						let dist = (dx * dx + dy * dy).sqrt();
						let c = if dist >= r_inner && dist <= r_outer {
							let mut theta = dx.atan2(-dy);
							if theta < 0.0 {
								theta += TAU;
							}
							if theta <= thresh { fill } else { TRACK }
						} else {
							bg
						};
						r += c[0] as f32;
						g += c[1] as f32;
						b += c[2] as f32;
						a += c[3] as f32;
					}
				}
				self.image
					.put_pixel(px as u32, py as u32, Rgba([(r / n) as u8, (g / n) as u8, (b / n) as u8, (a / n) as u8]));
			}
		}
		self
	}

	/// Encode the canvas as a `data:image/png;base64,...` URI for `set_image`.
	pub fn to_data_uri(&self) -> Result<String, String> {
		let mut buffer = Cursor::new(Vec::new());
		self.image.write_to(&mut buffer, image::ImageFormat::Png).map_err(|e| e.to_string())?;
		let b64 = general_purpose::STANDARD.encode(buffer.into_inner());
		Ok(format!("data:image/png;base64,{b64}"))
	}
}

impl Default for Canvas {
	fn default() -> Self {
		Self::new()
	}
}

/// Width in pixels that `text` would occupy at the given size.
pub fn text_width(text: &str, size: f32) -> i32 {
	let scaled = font().as_scaled(PxScale { x: size, y: size });
	let mut width = 0.0;
	let mut prev: Option<ab_glyph::GlyphId> = None;
	for c in text.chars() {
		let id = font().glyph_id(c);
		if let Some(p) = prev {
			width += scaled.kern(p, id);
		}
		width += scaled.h_advance(id);
		prev = Some(id);
	}
	width.ceil() as i32
}

/// Parse a `#RRGGBB` (or `RRGGBB`) hex string into an opaque color.
///
/// Returns `None` for anything that isn't exactly six hex digits, so callers can
/// fall back to a default (e.g. the threshold-based [`usage_color`]).
pub fn parse_hex_color(s: &str) -> Option<Rgba<u8>> {
	let s = s.trim().trim_start_matches('#');
	if s.len() != 6 || !s.is_ascii() {
		return None;
	}
	let r = u8::from_str_radix(&s[0..2], 16).ok()?;
	let g = u8::from_str_radix(&s[2..4], 16).ok()?;
	let b = u8::from_str_radix(&s[4..6], 16).ok()?;
	Some(Rgba([r, g, b, 255]))
}

/// Color for a utilization percentage: green / amber / red by threshold.
pub fn usage_color(percent: f32) -> Rgba<u8> {
	if percent >= 90.0 {
		Rgba([235, 60, 45, 255]) // red
	} else if percent >= 70.0 {
		Rgba([240, 175, 25, 255]) // amber
	} else {
		Rgba([55, 200, 110, 255]) // green
	}
}
