pub mod controls;
pub mod curve;
pub mod graphic;
pub mod presets;
pub mod theme;

use presets::PresetAction;

/// What the window asks of the app during a frame, besides direct edits to the settings.
#[derive(Default)]
pub struct Requests {
    pub preset: Option<PresetAction>,
    /// A message for the status line.
    pub notice: Option<String>,
    /// A message for an edit made this frame, offered with Undo.
    pub undoable: Option<String>,
    /// The Undo next to a message was clicked.
    pub undo: bool,
}

/// "80 Hz", "1.2 kHz".
pub fn format_freq(freq: f32) -> String {
    if freq < 1_000.0 {
        let decimals = if freq < 100.0 && freq.fract() != 0.0 { 1 } else { 0 };
        format!("{freq:.decimals$} Hz")
    } else {
        format!("{} kHz", trim_decimals(freq / 1_000.0, 2))
    }
}

/// A slider frequency as printed under it: "31.5", "250", "1k", "12.5k".
pub fn short_freq(freq: f32) -> String {
    if freq < 1_000.0 {
        trim_decimals(freq, 1)
    } else {
        format!("{}k", trim_decimals(freq / 1_000.0, 1))
    }
}

fn trim_decimals(value: f32, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// "+3.5 dB", "−2.0 dB" with a true minus sign, and "0 dB".
pub fn format_gain(db: f32) -> String {
    let tenths = (db * 10.0).round() as i32;
    match tenths {
        0 => "0 dB".into(),
        _ => {
            let sign = if tenths > 0 { '+' } else { '\u{2212}' };
            format!("{sign}{}.{} dB", tenths.abs() / 10, tenths.abs() % 10)
        }
    }
}

/// "L 30", "Centre", "R 30".
pub fn format_balance(balance: f32) -> String {
    let percent = (balance * 100.0).round() as i32;
    match percent {
        0 => "Centre".into(),
        p if p < 0 => format!("L {}", -p),
        p => format!("R {p}"),
    }
}

/// Rounds a frequency to three significant digits while dragging: 1234 Hz becomes 1230.
pub fn snap_freq(freq: f32) -> f32 {
    if freq <= 0.0 || !freq.is_finite() {
        return freq;
    }
    let step = 10f32.powi(freq.log10().floor() as i32 - 2);
    (freq / step).round() * step
}

/// Rounds a gain to a tenth of a dB, and to exactly 0 near it.
pub fn snap_gain(db: f32) -> f32 {
    if db.abs() <= 0.25 { 0.0 } else { (db * 10.0).round() / 10.0 }
}

/// A number as people type it: spaces ignored, a true minus or a hyphen, and a comma read
/// as a thousands separator before three digits ("1,500") and otherwise as a decimal
/// point ("1,5").
fn number(text: &str) -> Option<f64> {
    let text: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '\u{2212}' { '-' } else { c })
        .collect();
    let thousands = !text.contains('.')
        && text.contains(',')
        && text.split(',').skip(1).all(|group| group.len() == 3);
    let text = if thousands { text.replace(',', "") } else { text.replace(',', ".") };
    let valid = !text.is_empty()
        && text.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'));
    text.parse().ok().filter(|v: &f64| valid && v.is_finite())
}

/// A frequency: "1500", "1.5k", "1.5 kHz", "800 Hz".
pub fn parse_freq(text: &str) -> Option<f64> {
    let text = text.trim().to_lowercase();
    if let Some(khz) = text.strip_suffix("khz").or_else(|| text.strip_suffix('k')) {
        return number(khz).map(|v| v * 1_000.0);
    }
    number(text.strip_suffix("hz").unwrap_or(&text))
}

/// A gain: "-3", "+2.5 dB".
pub fn parse_gain(text: &str) -> Option<f64> {
    let text = text.trim().to_lowercase();
    number(text.strip_suffix("db").unwrap_or(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format_freq(80.0), "80 Hz");
        assert_eq!(format_freq(31.5), "31.5 Hz");
        assert_eq!(format_freq(1_250.0), "1.25 kHz");
        assert_eq!(format_freq(16_000.0), "16 kHz");
        assert_eq!(short_freq(31.5), "31.5");
        assert_eq!(short_freq(1_000.0), "1k");
        assert_eq!(short_freq(12_500.0), "12.5k");
        assert_eq!(format_gain(3.5), "+3.5 dB");
        assert_eq!(format_gain(-12.0), "\u{2212}12.0 dB");
        assert_eq!(format_gain(0.04), "0 dB");
        assert_eq!(format_balance(-0.3), "L 30");
        assert_eq!(format_balance(0.004), "Centre");
        assert_eq!(format_balance(1.0), "R 100");
    }

    #[test]
    fn snapping() {
        assert_eq!(snap_freq(1_234.5), 1_230.0);
        assert!((snap_freq(31.47) - 31.5).abs() < 1e-4);
        assert_eq!(snap_freq(19_987.0), 20_000.0);
        assert_eq!(snap_gain(0.2), 0.0);
        assert_eq!(snap_gain(-0.26), -0.3);
        assert_eq!(snap_gain(3.46), 3.5);
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_freq("1.5k"), Some(1_500.0));
        assert_eq!(parse_freq(" 1,5 kHz "), Some(1_500.0));
        assert_eq!(parse_freq("800 Hz"), Some(800.0));
        assert_eq!(parse_freq("1500"), Some(1_500.0));
        assert_eq!(parse_freq("1,500 Hz"), Some(1_500.0));
        assert_eq!(parse_freq("12,500"), Some(12_500.0));
        assert_eq!(parse_freq("1,5k"), Some(1_500.0));
        assert_eq!(parse_freq("abc"), None);
        assert_eq!(parse_freq("1e3"), None);
        assert_eq!(parse_freq("NaN"), None);
        assert_eq!(parse_gain("\u{2212}3 dB"), Some(-3.0));
        assert_eq!(parse_gain("+2.5"), Some(2.5));
        assert_eq!(parse_gain(""), None);
    }
}

#[cfg(test)]
mod frame_tests {
    use super::controls::{self, Level, Status};
    use super::{Requests, curve, presets};
    use crate::state::{Mode, Settings};
    use eframe::egui::{self, CentralPanel, Panel, RawInput, Rect, pos2, vec2};

    /// Lays out the whole window for a few frames at a given size, as the app does, and
    /// returns the height the curve was left with.
    fn frames(settings: &mut Settings, selected: &mut Option<usize>, size: egui::Vec2) -> f32 {
        let ctx = egui::Context::default();
        // The app's own fonts and spacing, which decide how tall each row is.
        super::theme::install(&ctx);
        let mut curve_height = 0.0;
        for frame in 0..60 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
                time: Some(frame as f64 / 60.0),
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let mut requests = Requests::default();
                let level = Level { output_db: -12.0, clipped: frame == 1 };
                let status = Status { text: "Speakers · 48.0 kHz".into(), undo: frame == 2 };
                Panel::top("top").show(ui, |ui| {
                    controls::top_bar(ui, settings, &level, &status, 48_000.0, &mut requests);
                });
                Panel::bottom("bands").resizable(true).default_size(300.0).min_size(200.0).show(ui, |ui| {
                    controls::editor(ui, &mut settings.eq, selected, &mut requests);
                });
                curve_height = CentralPanel::default()
                    .show(ui, |ui| {
                        curve::show(ui, &mut settings.eq, selected, 48_000.0, true, &mut requests);
                    })
                    .response
                    .rect
                    .height();
            });
            // Nothing draws here, so the font atlas updates are dropped on purpose.
            output.textures_delta.clear();
        }
        curve_height
    }

    #[test]
    fn every_view_lays_out_without_panicking() {
        for size in [vec2(920.0, 640.0), vec2(640.0, 440.0), vec2(1800.0, 1100.0)] {
            let mut settings = Settings::default();
            settings.save_preset("Mine").unwrap();
            let mut selected = Some(2);
            settings.eq.bass_db = 6.0;
            settings.eq.clip_protection = false;
            let curve = frames(&mut settings, &mut selected, size);
            println!("{size:?}: curve {curve}");
            // The bands must leave the curve most of the window.
            assert!(curve >= size.y * 0.3, "{size:?}: curve squeezed to {curve}");

            settings.eq.bands.clear();
            frames(&mut settings, &mut selected, size);

            settings.eq.mode = Mode::Graphic;
            for count in [5, 10, 15, 31] {
                settings.eq.graphic.set_band_count(count);
                settings.eq.graphic.gains_db.fill(12.0);
                let curve = frames(&mut settings, &mut None, size);
                println!("{size:?} graphic {count}: curve {curve}");
                assert!(curve >= size.y * 0.3, "{size:?}: curve squeezed to {curve}");
            }
            settings.eq.bypass = true;
            frames(&mut settings, &mut None, size);
            assert_eq!(presets::active_user_preset(&settings), Some(4));
        }
    }
}
