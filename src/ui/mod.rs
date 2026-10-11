pub mod controls;
pub mod curve;
pub mod presets;
pub mod theme;

/// "80 Hz", "1.2 kHz".
pub fn format_freq(freq: f32) -> String {
    if freq < 1_000.0 {
        format!("{freq:.0} Hz")
    } else {
        let khz = format!("{:.2}", freq / 1_000.0);
        format!("{} kHz", khz.trim_end_matches('0').trim_end_matches('.'))
    }
}
