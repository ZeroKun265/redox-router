//! Volume slider math (std only). PipeWire stores volume as a linear gain; a
//! cubic curve (the same one wpctl and most mixers use) feels natural: the
//! slider value `s` maps to linear gain `s^3`, so 100% is gain 1.0.

/// Slider range: 0.0 ..= SLIDER_MAX (150%).
pub const SLIDER_MAX: f32 = 1.5;

pub fn slider_to_linear(s: f32) -> f32 {
    s.max(0.0).powi(3)
}

pub fn linear_to_slider(linear: f32) -> f32 {
    linear.max(0.0).cbrt()
}

pub fn percent(s: f32) -> u32 {
    (s * 100.0).round().max(0.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_gain_is_100_percent() {
        assert_eq!(percent(linear_to_slider(1.0)), 100);
        assert_eq!(slider_to_linear(1.0), 1.0);
    }

    #[test]
    fn curve_round_trips() {
        for s in [0.0, 0.1, 0.5, 0.9, 1.0, 1.5] {
            assert!((linear_to_slider(slider_to_linear(s)) - s).abs() < 1e-4);
        }
        assert!((slider_to_linear(0.5) - 0.125).abs() < 1e-6);
    }

    #[test]
    fn negative_input_is_clamped() {
        assert_eq!(slider_to_linear(-1.0), 0.0);
        assert_eq!(linear_to_slider(-0.5), 0.0);
        assert_eq!(percent(-0.2), 0);
    }
}
