use colored::Color;

/// Golden angle in degrees — maximizes hue separation between consecutive indices.
const GOLDEN_ANGLE: f64 = 137.508;
const COLOR_SATURATION: f64 = 0.7;
const COLOR_LIGHTNESS: f64 = 0.6;

/// Convert HSL values (h: 0..360, s: 0..1, l: 0..1) to RGB bytes.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
  let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
  let h2 = h / 60.0;
  let x = c * (1.0 - (h2 % 2.0 - 1.0).abs());
  let (r1, g1, b1) = match h2 as u32 {
    0 => (c, x, 0.0),
    1 => (x, c, 0.0),
    2 => (0.0, c, x),
    3 => (0.0, x, c),
    4 => (x, 0.0, c),
    _ => (c, 0.0, x),
  };
  let m = l - c / 2.0;
  (
    ((r1 + m) * 255.0) as u8,
    ((g1 + m) * 255.0) as u8,
    ((b1 + m) * 255.0) as u8,
  )
}

/// Generate a distinct color for a process by its index using the golden angle
/// to spread hues evenly around the color wheel.
pub fn color_for_index(index: usize) -> Color {
  let hue = (index as f64 * GOLDEN_ANGLE) % 360.0;
  let (r, g, b) = hsl_to_rgb(hue, COLOR_SATURATION, COLOR_LIGHTNESS);
  Color::TrueColor { r, g, b }
}
