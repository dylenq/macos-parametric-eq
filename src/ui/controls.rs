use super::band_color;
use crate::state::{
    Band, EqState, FilterKind, MAX_BANDS, MAX_FREQ, MAX_GAIN_DB, MAX_Q, MIN_FREQ, MIN_Q,
};
use eframe::egui::{
    Color32, ComboBox, DragValue, Grid, RichText, ScrollArea, Sense, Slider, Stroke, Ui, vec2,
};

/// Output level in dB with the time the signal last exceeded full scale.
pub struct Level {
    pub output_db: f32,
    pub clipped: bool,
}

/// Bypass, preamp, level meter and device status.
pub fn top_bar(ui: &mut Ui, state: &mut EqState, level: &Level, status: &str) {
    ui.horizontal(|ui| {
        let label = if state.bypass { "Bypassed" } else { "EQ on" };
        if ui
            .selectable_label(!state.bypass, label)
            .on_hover_text("Toggle bypass to compare with the unprocessed sound")
            .clicked()
        {
            state.bypass = !state.bypass;
        }
        ui.separator();
        ui.label("Preamp");
        ui.add(
            Slider::new(&mut state.preamp_db, -MAX_GAIN_DB..=MAX_GAIN_DB)
                .suffix(" dB")
                .fixed_decimals(1),
        )
        .on_hover_text("Lower this if boosts make the output clip");
        ui.separator();
        meter(ui, level);
        ui.separator();
        ui.label(RichText::new(status).weak());
    });
}

fn meter(ui: &mut Ui, level: &Level) {
    const FLOOR_DB: f32 = -60.0;
    let (rect, response) = ui.allocate_exact_size(vec2(120.0, 10.0), Sense::hover());
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let fraction = ((level.output_db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
    let mut filled = rect;
    filled.set_width(rect.width() * fraction);
    let color = if level.clipped {
        visuals.error_fg_color
    } else {
        visuals.selection.stroke.color
    };
    painter.rect_filled(filled, 2.0, color);
    response.on_hover_text(format!("Output peak {:.1} dB", level.output_db));
    if level.clipped {
        ui.label(RichText::new("CLIP").color(visuals.error_fg_color).strong());
    }
}

/// One editable row per band.
pub fn band_list(ui: &mut Ui, state: &mut EqState, selected: &mut Option<usize>) {
    let mut delete = None;
    ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        Grid::new("bands").num_columns(7).spacing([12.0, 6.0]).show(ui, |ui| {
            for (index, band) in state.bands.iter_mut().enumerate() {
                if band_row(ui, index, band, *selected == Some(index)) {
                    delete = Some(index);
                }
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(state.bands.len() < MAX_BANDS, |ui| {
                if ui.button("Add band").clicked() {
                    let mut band = Band::new(FilterKind::Peak, 1_000.0);
                    band.q = 1.0;
                    state.bands.push(band);
                    *selected = Some(state.bands.len() - 1);
                }
            });
            if ui
                .button("Reset")
                .on_hover_text("Restore the default flat bands")
                .clicked()
            {
                *state = EqState {
                    bypass: state.bypass,
                    ..EqState::default()
                };
                *selected = None;
            }
        });
    });
    if let Some(index) = delete {
        state.bands.remove(index);
        *selected = None;
    }
}

/// Returns true if the band should be deleted.
fn band_row(ui: &mut Ui, index: usize, band: &mut Band, selected: bool) -> bool {
    let color = band_color(index);
    let (dot, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    ui.painter().circle_filled(dot.center(), 8.0, color);
    if selected {
        ui.painter().circle_stroke(dot.center(), 10.5, Stroke::new(1.5, color));
    }
    ui.painter().text(
        dot.center(),
        eframe::egui::Align2::CENTER_CENTER,
        (index + 1).to_string(),
        eframe::egui::FontId::proportional(10.0),
        Color32::BLACK,
    );

    ui.checkbox(&mut band.enabled, "");
    ComboBox::from_id_salt(("kind", index))
        .selected_text(band.kind.label())
        .width(96.0)
        .show_ui(ui, |ui| {
            for kind in FilterKind::ALL {
                ui.selectable_value(&mut band.kind, kind, kind.label());
            }
        });
    let (freq_speed, q_speed) = (band.freq * 0.01, band.q * 0.01);
    ui.add(
        DragValue::new(&mut band.freq)
            .range(MIN_FREQ..=MAX_FREQ)
            .speed(freq_speed)
            .max_decimals(0)
            .suffix(" Hz"),
    );
    ui.add_enabled(
        band.kind.has_gain(),
        DragValue::new(&mut band.gain_db)
            .range(-MAX_GAIN_DB..=MAX_GAIN_DB)
            .speed(0.1)
            .fixed_decimals(1)
            .suffix(" dB"),
    );
    ui.add(
        DragValue::new(&mut band.q)
            .range(MIN_Q..=MAX_Q)
            .speed(q_speed)
            .fixed_decimals(2)
            .prefix("Q "),
    );
    ui.small_button("Delete").clicked()
}
