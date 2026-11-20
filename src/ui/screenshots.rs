//! The screenshots in the README and on the website, rendered from the real window layout
//! without opening a window or touching audio:
//!
//! ```sh
//! cargo test --release screenshots -- --ignored
//! ```
//!
//! Writes docs/screenshot-{light,dark}.png, docs/screenshot-graphic-{light,dark}.png and
//! docs/screenshot-sweep-{light,dark}.png.

use super::controls::{Level, Status};
use super::{AbView, View, theme};
use crate::state::{Band, EqState, FilterKind, Mode, Settings, builtins};
use eframe::egui::{self, Color32, Id, LayerId, Order, Stroke, Theme, pos2, vec2};
use egui_kittest::Harness;
use image::RgbaImage;

/// The window's default size in points, drawn at Retina scale.
const SIZE: egui::Vec2 = vec2(1120.0, 640.0);
const SCALE: f32 = 2.0;
/// Window corner radius in points, as macOS draws it.
const CORNER: f32 = 20.5;
/// The close, minimize and zoom buttons: fill and rim.
const BUTTONS: [(Color32, Color32); 3] = [
    (Color32::from_rgb(0xff, 0x5f, 0x57), Color32::from_rgb(0xe0, 0x44, 0x3e)),
    (Color32::from_rgb(0xfe, 0xbc, 0x2e), Color32::from_rgb(0xde, 0xa1, 0x23)),
    (Color32::from_rgb(0x28, 0xc8, 0x40), Color32::from_rgb(0x1a, 0xab, 0x29)),
];

/// What a picture shows: the settings, the selected band, a sweep in progress as
/// (frequency, fraction played), and the sound remembered as A, if they are.
type Scene = (Settings, Option<usize>, Option<(f32, f32)>, Option<EqState>);

/// A five-band correction like the one people make for a pair of speakers.
fn parametric() -> Scene {
    let mut settings = Settings::default();
    let band = |kind, freq, gain_db, q| Band { kind, freq, gain_db, q, enabled: true };
    settings.eq.bands = vec![
        band(FilterKind::LowShelf, 90.0, 5.0, 0.71),
        band(FilterKind::Peak, 280.0, -3.5, 1.2),
        band(FilterKind::Peak, 1_200.0, 2.0, 1.0),
        band(FilterKind::Peak, 3_500.0, -4.5, 2.0),
        band(FilterKind::HighShelf, 9_000.0, 4.0, 0.71),
    ];
    settings.eq.preamp_db = -5.0;
    settings.save_preset("Studio Monitors").unwrap();
    (settings, Some(2), None, None)
}

/// A sweep most of the way up, with three peaks marked, a cut placed on one of them, and
/// the sound from before the cut remembered as A to compare with.
fn sweep() -> Scene {
    let (mut settings, ..) = parametric();
    let before = settings.eq.clone();
    settings.eq.bands[3] = Band { kind: FilterKind::Peak, freq: 2_800.0, gain_db: -3.0, q: 4.0, enabled: true };
    settings.save_preset("Studio Monitors").unwrap();
    settings.eq.sweep.enabled = true;
    settings.eq.sweep.duration = 8.0;
    for freq in [180.0, 2_800.0, 7_400.0] {
        crate::state::add_mark(settings.eq_and_marks().1, freq);
    }
    (settings, Some(3), Some((5_200.0, 0.8)), Some(before))
}

/// The built-in Vocal preset on the graphic sliders.
fn graphic() -> Scene {
    let mut settings = Settings::default();
    let vocal = builtins().iter().position(|p| p.name == "Vocal").unwrap();
    settings.apply_preset(vocal);
    assert_eq!(settings.eq.mode, Mode::Graphic);
    (settings, None, None, None)
}

/// Draws the window as the app would show it and returns the picture.
fn render(look: Theme, (mut settings, mut selected, sweep, reference): Scene) -> RgbaImage {
    let mut installed = false;
    // The harness puts this much margin around the closure's ui, which is cropped off after.
    const MARGIN: f32 = 8.0;
    let mut harness = Harness::builder()
        .with_size(SIZE + vec2(2.0 * MARGIN, 2.0 * MARGIN))
        .with_pixels_per_point(SCALE)
        .with_theme(look)
        .with_os(egui::os::OperatingSystem::Mac)
        .wgpu()
        .build_ui(move |ui| {
            if !std::mem::replace(&mut installed, true) {
                theme::install_default_accent(ui.ctx());
            }
            let view = View {
                level: Level { output_db: -14.0, clipped: false },
                status: Status { text: "MacBook Pro Speakers · 48.0 kHz".into(), undo: false },
                silent: false,
                sample_rate: 48_000.0,
                keys: false,
                sweep,
                sweep_open: false,
                sweep_ready: true,
                ab: reference.as_ref().map(|reference| AbView {
                    hearing_a: false,
                    trims: crate::ab::Ab::new(reference).trims(&settings.eq, 48_000.0),
                    reference: reference.clone(),
                }),
            };
            super::window(ui, &mut settings, &mut selected, &view);
            // macOS draws these over the transparent title bar.
            let painter = ui.ctx().layer_painter(LayerId::new(Order::Foreground, Id::new("window-buttons")));
            for (i, (fill, rim)) in BUTTONS.into_iter().enumerate() {
                let center = pos2(MARGIN + 15.75 + 23.0 * i as f32, MARGIN + 15.75);
                painter.circle(center, 6.75, fill, Stroke::new(0.5, rim));
            }
        });
    // Fonts load on the first frame, and the graph eases into its scale.
    harness.run_steps(8);
    let rendered = harness.render().expect("wgpu could render the window");
    let edge = (MARGIN * SCALE) as u32;
    let (w, h) = ((SIZE.x * SCALE) as u32, (SIZE.y * SCALE) as u32);
    let mut image = image::imageops::crop_imm(&rendered, edge, edge, w, h).to_image();
    round_corners(&mut image, CORNER * SCALE);
    image
}

/// Makes the corners transparent outside a rounded rectangle, with smooth edges.
fn round_corners(image: &mut RgbaImage, radius: f32) {
    let (w, h) = (image.width() as f32, image.height() as f32);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let (cx, cy) = (px.clamp(radius, w - radius), py.clamp(radius, h - radius));
        let distance = (px - cx).hypot(py - cy);
        let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
        pixel[3] = (pixel[3] as f32 * coverage).round() as u8;
    }
}

#[test]
#[ignore = "writes the screenshots in docs/; run with --ignored"]
fn screenshots() {
    let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    let scenes = [("screenshot", parametric as fn() -> Scene), ("screenshot-graphic", graphic), ("screenshot-sweep", sweep)];
    for (name, scene) in scenes {
        for (look, suffix) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
            let image = render(look, scene());
            assert_eq!(image.dimensions(), ((SIZE.x * SCALE) as u32, (SIZE.y * SCALE) as u32));
            image.save(docs.join(format!("{name}-{suffix}.png"))).unwrap();
        }
    }
}
