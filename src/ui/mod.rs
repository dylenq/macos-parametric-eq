pub mod controls;
pub mod curve;

use eframe::egui::Color32;

const BAND_COLORS: [Color32; 8] = [
    Color32::from_rgb(255, 122, 122),
    Color32::from_rgb(255, 176, 92),
    Color32::from_rgb(240, 214, 90),
    Color32::from_rgb(132, 214, 122),
    Color32::from_rgb(98, 208, 208),
    Color32::from_rgb(112, 168, 255),
    Color32::from_rgb(176, 142, 255),
    Color32::from_rgb(240, 138, 214),
];

pub fn band_color(index: usize) -> Color32 {
    BAND_COLORS[index % BAND_COLORS.len()]
}
