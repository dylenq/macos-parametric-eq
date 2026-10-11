use super::presets::{self, PresetAction};
use super::theme::{self, Palette};
use crate::state::{
    Band, EqState, FilterKind, Settings, MAX_BANDS, MAX_FREQ, MAX_GAIN_DB, MAX_Q, MIN_FREQ, MIN_Q,
};
use eframe::egui::{
    Align, Button, Checkbox, Color32, ComboBox, DragValue, Frame, Label, Layout, Margin, Response,
    RichText, ScrollArea, Sense, Slider, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

const ROW_HEIGHT: f32 = 22.0;
/// Column widths shared by the header and every band row.
const NUMBER_WIDTH: f32 = 18.0;
const TYPE_WIDTH: f32 = 116.0;
const VALUE_WIDTH: f32 = 84.0;
const METER_WIDTH: f32 = 96.0;
/// Room kept for the "Clipping" label so the status text does not jump when it appears.
const CLIPPING_WIDTH: f32 = 52.0;

/// Output level in dB and whether the signal recently exceeded full scale.
pub struct Level {
    pub output_db: f32,
    pub clipped: bool,
}

/// Equalizer switch, preamp and presets, and on the trailing side the output level and
/// device.
pub fn top_bar(
    ui: &mut Ui,
    settings: &mut Settings,
    level: &Level,
    status: &str,
) -> Option<PresetAction> {
    let p = &theme::palette(ui);
    let mut action = None;
    ui.horizontal(|ui| {
        let state = &mut settings.eq;
        let mut on = !state.bypass;
        let toggled = switch(ui, &mut on, p)
            .on_hover_text("Turn off to compare with the unprocessed sound")
            .changed();
        // The label toggles too, like a system checkbox title.
        let label = ui.add(Label::new("Equalizer").sense(Sense::click()));
        if toggled || label.clicked() {
            state.bypass = !state.bypass;
        }

        ui.add_space(16.0);
        ui.label(RichText::new("Preamp").color(p.secondary));
        // Shorter than elsewhere, to leave the device name room.
        ui.spacing_mut().slider_width = 120.0;
        ui.add(
            Slider::new(&mut state.preamp_db, -MAX_GAIN_DB..=MAX_GAIN_DB)
                .suffix(" dB")
                .fixed_decimals(1),
        )
        .on_hover_text("Lower this if boosts make the output clip");

        ui.add_space(8.0);
        action = presets::picker(ui, settings);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // The device name gives way first when the window is narrow.
            let reserved = METER_WIDTH + CLIPPING_WIDTH + 32.0;
            ui.scope(|ui| {
                ui.set_max_width((ui.available_width() - reserved).max(0.0));
                ui.add(Label::new(RichText::new(status).color(p.secondary)).truncate())
                    .on_hover_text(status);
            });
            ui.add_space(8.0);
            meter(ui, level, p);
            if level.clipped {
                // Text as well as color, so clipping never depends on seeing red.
                ui.label(RichText::new("Clipping").color(p.danger));
            }
        });
    });
    action
}

/// A compact on/off switch in the system style.
fn switch(ui: &mut Ui, on: &mut bool, p: &Palette) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let value = *on;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, value, "Equalizer"));

    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let track = if response.hovered() { p.control_active } else { p.control };
    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() / 2.0, track.lerp_to_gamma(p.accent, t));
    let radius = rect.height() / 2.0 - 2.0;
    let x = rect.left() + rect.height() / 2.0 + t * (rect.width() - rect.height());
    painter.circle_filled(pos2(x, rect.center().y), radius, Color32::WHITE);
    response
}

fn meter(ui: &mut Ui, level: &Level, p: &Palette) {
    const FLOOR_DB: f32 = -60.0;
    let (rect, response) = ui.allocate_exact_size(vec2(METER_WIDTH, 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, p.control);
    let fraction = ((level.output_db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
    let mut filled = rect;
    filled.set_width(rect.width() * fraction);
    let color = if level.clipped { p.danger } else { p.accent };
    painter.rect_filled(filled, 2.0, color);
    response.on_hover_text(format!("Output peak {:.1} dB", level.output_db.max(FLOOR_DB)));
}

fn row_frame(fill: Color32) -> Frame {
    Frame::new()
        .fill(fill)
        .corner_radius(6)
        .inner_margin(Margin::symmetric(8, 3))
}

/// One editable row per band under a header that names the columns.
pub fn band_list(ui: &mut Ui, state: &mut EqState, selected: &mut Option<usize>) {
    let p = &theme::palette(ui);
    let mut delete = None;

    row_frame(Color32::TRANSPARENT).show(ui, |ui| {
        ui.horizontal(|ui| {
            let header = |ui: &mut Ui, width: f32, text: &str| {
                ui.add_sized(
                    [width, 14.0],
                    Label::new(RichText::new(text).small().color(p.secondary)),
                );
            };
            header(ui, NUMBER_WIDTH, "");
            header(ui, ROW_HEIGHT, "On");
            header(ui, TYPE_WIDTH, "Type");
            header(ui, VALUE_WIDTH, "Frequency");
            header(ui, VALUE_WIDTH, "Gain");
            header(ui, VALUE_WIDTH, "Q");
        });
    });

    ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (index, band) in state.bands.iter_mut().enumerate() {
            let fill = if *selected == Some(index) {
                p.row_selected
            } else {
                Color32::TRANSPARENT
            };
            let row = row_frame(fill)
                .show(ui, |ui| ui.horizontal(|ui| band_row(ui, index, band, p)).inner)
                .inner;
            if row.touched {
                *selected = Some(index);
            }
            if row.delete {
                delete = Some(index);
            }
        }

        ui.add_space(8.0);
        row_frame(Color32::TRANSPARENT).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(state.bands.len() < MAX_BANDS, |ui| {
                    if ui.button("Add Band").clicked() {
                        state.add_band(selected);
                    }
                });
                if ui
                    .button("Reset")
                    .on_hover_text("Restore the default flat bands. Undo with ⌘Z.")
                    .clicked()
                {
                    state.reset_bands();
                    *selected = None;
                }
                ui.add_space(8.0);
                ui.checkbox(&mut state.clip_protection, "Clip Protection").on_hover_text(
                    "Turns the output down for a moment when boosts would make it clip",
                );
            });
        });
    });

    if let Some(index) = delete {
        state.bands.remove(index);
        *selected = None;
    }
}

struct RowResult {
    /// The row was interacted with and should become the selection.
    touched: bool,
    delete: bool,
}

fn band_row(ui: &mut Ui, index: usize, band: &mut Band, p: &Palette) -> RowResult {
    let mut touched = false;
    let mut track = |response: &Response| {
        touched |= response.clicked() || response.dragged() || response.gained_focus();
    };

    track(&ui.add_sized(
        [NUMBER_WIDTH, ROW_HEIGHT],
        Label::new(RichText::new((index + 1).to_string()).color(p.secondary))
            .sense(Sense::click()),
    ));
    ui.scope(|ui| {
        // A tighter radius than buttons, so the box reads as a checkbox and not a radio button.
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
            state.corner_radius = 4.into();
        }
        track(&ui.add_sized(
            [ROW_HEIGHT, ROW_HEIGHT],
            Checkbox::without_text(&mut band.enabled),
        ));
    });
    let kind = ComboBox::from_id_salt(("kind", index))
        .selected_text(band.kind.label())
        .width(TYPE_WIDTH)
        .show_ui(ui, |ui| {
            for kind in FilterKind::ALL {
                ui.selectable_value(&mut band.kind, kind, kind.label());
            }
        });
    track(&kind.response);

    let (freq_speed, q_speed) = (band.freq * 0.01, band.q * 0.01);
    track(&ui.add_sized(
        [VALUE_WIDTH, ROW_HEIGHT],
        DragValue::new(&mut band.freq)
            .range(MIN_FREQ..=MAX_FREQ)
            .speed(freq_speed)
            .max_decimals(0)
            .suffix(" Hz"),
    ));
    ui.add_enabled_ui(band.kind.has_gain(), |ui| {
        track(&ui.add_sized(
            [VALUE_WIDTH, ROW_HEIGHT],
            DragValue::new(&mut band.gain_db)
                .range(-MAX_GAIN_DB..=MAX_GAIN_DB)
                .speed(0.1)
                .fixed_decimals(1)
                .suffix(" dB"),
        ));
    });
    track(&ui.add_sized(
        [VALUE_WIDTH, ROW_HEIGHT],
        DragValue::new(&mut band.q)
            .range(MIN_Q..=MAX_Q)
            .speed(q_speed)
            .fixed_decimals(2),
    ));

    let delete = ui
        .add_sized(
            [ROW_HEIGHT, ROW_HEIGHT],
            Button::new(RichText::new("−").color(p.secondary)).frame(false),
        )
        .on_hover_text("Delete Band")
        .clicked();
    RowResult { touched, delete }
}
