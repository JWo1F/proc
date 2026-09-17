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

#[cfg(test)]
mod tests {
  use super::*;

  fn rgb(color: Color) -> (u8, u8, u8) {
    match color {
      Color::TrueColor { r, g, b } => (r, g, b),
      other => panic!("expected a TrueColor, got {:?}", other),
    }
  }

  #[test]
  fn the_same_index_always_gives_the_same_color() {
    assert_eq!(color_for_index(7), color_for_index(7));
  }

  #[test]
  fn the_first_colors_are_all_distinct() {
    let colors: Vec<_> = (0..32).map(|i| rgb(color_for_index(i))).collect();
    let mut unique = colors.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), colors.len(), "{:?}", colors);
  }

  #[test]
  fn neighboring_indices_are_far_apart_in_hue() {
    // The golden angle puts consecutive processes on opposite sides of the
    // wheel, which is the whole point of using it over a sequential step.
    let (r0, g0, b0) = rgb(color_for_index(0));
    let (r1, g1, b1) = rgb(color_for_index(1));
    let distance =
      (r0 as i32 - r1 as i32).abs() + (g0 as i32 - g1 as i32).abs() + (b0 as i32 - b1 as i32).abs();
    assert!(distance > 100, "colors too close: {}", distance);
  }

  #[test]
  fn every_channel_stays_in_the_mid_range_for_readability() {
    // Saturation 0.7 / lightness 0.6 should never produce black or white,
    // which would be invisible against a terminal background.
    for index in 0..360 {
      let (r, g, b) = rgb(color_for_index(index));
      let max = r.max(g).max(b);
      let min = r.min(g).min(b);
      assert!(max > 100, "index {} too dark: {:?}", index, (r, g, b));
      assert!(min < 240, "index {} too washed out: {:?}", index, (r, g, b));
    }
  }

  #[test]
  fn hue_wraps_around_the_wheel() {
    // 360 / 137.508 is not a whole number, so no index repeats index 0 early,
    // but the hue itself must stay inside one turn.
    for index in 0..1000 {
      let hue = (index as f64 * GOLDEN_ANGLE) % 360.0;
      assert!((0.0..360.0).contains(&hue), "index {} hue {}", index, hue);
    }
  }

  #[test]
  fn hsl_to_rgb_covers_every_sector() {
    // One hue per 60-degree sector, so each arm of the match is exercised.
    let reds = hsl_to_rgb(0.0, 1.0, 0.5);
    assert_eq!(reds, (255, 0, 0));
    assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), (0, 255, 0));
    assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), (0, 0, 255));
    for hue in [30.0, 90.0, 150.0, 210.0, 270.0, 330.0] {
      let (r, g, b) = hsl_to_rgb(hue, 1.0, 0.5);
      assert!(r > 0 || g > 0 || b > 0, "hue {} produced black", hue);
    }
  }

  #[test]
  fn zero_saturation_is_gray() {
    let (r, g, b) = hsl_to_rgb(200.0, 0.0, 0.5);
    assert_eq!(r, g);
    assert_eq!(g, b);
  }
}
