use super::presets;
use super::theme::{self, Palette};
use super::{Requests, format_balance, format_gain, graphic, parse_freq, parse_gain};
use crate::dsp::graphic::COUNTS;
use crate::dsp::response;
use crate::state::{
    Band, EqState, FilterKind, MAX_BANDS, MAX_FREQ, MAX_GAIN_DB, MAX_Q, MIN_FREQ, MIN_Q, Mode,
    SLIDER_MAX_DB, Settings, snap_balance,
};
use eframe::egui::{
    Align, Button, CentralPanel, Checkbox, Color32, ComboBox, DragValue, Frame, Label, Layout,
    Margin, Panel, Response, RichText, ScrollArea, Sense, Slider, Ui, WidgetInfo, WidgetType, pos2,
    vec2,
};
use std::cell::Cell;

const ROW_HEIGHT: f32 = 22.0;
/// Column widths shared by the header and every band row.
const NUMBER_WIDTH: f32 = 18.0;
const TYPE_WIDTH: f32 = 116.0;
const VALUE_WIDTH: f32 = 84.0;
const METER_WIDTH: f32 = 96.0;
/// Room kept for the "Clipping" label so the status text does not jump when it appears.
const CLIPPING_WIDTH: f32 = 52.0;
const TONE_SLIDER_WIDTH: f32 = 120.0;
/// Over full scale by less than this still counts as fitting.
const OVER_THRESHOLD_DB: f32 = 0.05;
/// The suggested preamp is offered when it differs from the preamp by at least this.
const SUGGEST_THRESHOLD_DB: f32 = 0.5;
const FULL_MESSAGE: &str = "All 16 bands are in use. Remove one to add another.";

/// Output level in dB and whether the signal recently exceeded full scale.
pub struct Level {
    pub output_db: f32,
    pub clipped: bool,
}

/// The status line: the output device, or a message that may offer Undo.
pub struct Status {
    pub text: String,
    pub undo: bool,
}

/// Equalizer switch, preamp, headroom and presets, and on the trailing side the output
/// level and device.
pub fn top_bar(
    ui: &mut Ui,
    settings: &mut Settings,
    level: &Level,
    status: &Status,
    sample_rate: f64,
    requests: &mut Requests,
) {
    let p = &theme::palette(ui);
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
        ui.spacing_mut().slider_width = 110.0;
        let preamp = ui
            .add(
                Slider::new(&mut state.preamp_db, -MAX_GAIN_DB..=MAX_GAIN_DB)
                    .suffix(" dB")
                    .fixed_decimals(1),
            )
            .on_hover_text("Lower this if boosts make the output clip. Double-click for 0 dB.");
        if preamp.double_clicked() {
            state.preamp_db = 0.0;
        }
        headroom(ui, state, sample_rate, p);

        ui.add_space(8.0);
        requests.preset = presets::picker(ui, settings);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // The device name gives way first when the window is narrow.
            let reserved = METER_WIDTH + CLIPPING_WIDTH + 32.0;
            ui.scope(|ui| {
                ui.set_max_width((ui.available_width() - reserved).max(0.0));
                if status.undo && ui.link("Undo").clicked() {
                    requests.undo = true;
                }
                ui.add(Label::new(RichText::new(&status.text).color(p.secondary)).truncate())
                    .on_hover_text(&status.text);
            });
            ui.add_space(8.0);
            meter(ui, level, p);
            if level.clipped {
                // Text as well as color, so clipping never depends on seeing red.
                ui.label(RichText::new("Clipping").color(p.danger));
            }
        });
    });
}

/// How far the loudest boost stays under full scale with this preamp, and a preamp that
/// would keep it there.
fn headroom(ui: &mut Ui, state: &mut EqState, sample_rate: f64, p: &Palette) {
    if state.bypass {
        return;
    }
    let peak = response::peak_db(state, sample_rate);
    let headroom = -(peak + state.preamp_db);
    if headroom < -OVER_THRESHOLD_DB {
        let (color, hint) = if state.clip_protection {
            (p.secondary, "Clip Protection turns the output down while boosts go over full scale")
        } else {
            (p.danger, "The loudest boost goes past full scale, so loud audio will clip")
        };
        ui.label(RichText::new(format!("Over by {:.1} dB", -headroom)).color(color))
            .on_hover_text(hint);
    } else {
        ui.label(
            RichText::new(format!("Headroom {:.1} dB", headroom.max(0.0))).color(p.secondary),
        )
        .on_hover_text("How far the loudest boost stays under full scale");
    }
    let suggested = response::suggested_preamp_db(peak);
    if suggested < 0.0
        && (suggested - state.preamp_db).abs() >= SUGGEST_THRESHOLD_DB
        && ui
            .small_button(format!("Apply {}", format_gain(suggested)))
            .on_hover_text("Set the preamp so the loudest boost just reaches full scale")
            .clicked()
    {
        state.preamp_db = suggested;
    }
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

/// The lower half of the window: the mode, the bands or sliders, the buttons that act on
/// them, and bass, treble and balance.
pub fn editor(ui: &mut Ui, state: &mut EqState, selected: &mut Option<usize>, requests: &mut Requests) {
    let p = &theme::palette(ui);
    row_frame(Color32::TRANSPARENT).show(ui, |ui| {
        ui.horizontal(|ui| {
            let before = state.mode;
            ui.selectable_value(&mut state.mode, Mode::Parametric, "Parametric")
                .on_hover_text("Up to 16 bands of any type (⌘1)");
            ui.selectable_value(&mut state.mode, Mode::Graphic, "Graphic")
                .on_hover_text("Fixed bands with a slider each (⌘2)");
            if state.mode != before {
                *selected = None;
            }
            ui.add_space(12.0);
            match state.mode {
                Mode::Graphic => {
                    let mut count = state.graphic.band_count;
                    ComboBox::from_id_salt("band-count")
                        .selected_text(format!("{count} bands"))
                        .show_ui(ui, |ui| {
                            for n in COUNTS {
                                ui.selectable_value(&mut count, n, format!("{n} bands"));
                            }
                        })
                        .response
                        .on_hover_text("Number of sliders. The curve keeps its shape.");
                    state.graphic.set_band_count(count);
                }
                Mode::Parametric => {
                    ui.label(
                        RichText::new(format!("{} of {MAX_BANDS} bands", state.bands.len()))
                            .color(p.secondary),
                    );
                }
            }
        });
    });

    // The buttons and tone controls take the height they need at the bottom, and the bands
    // or sliders fill what is left. Sizing either by a guess would let the panel outgrow
    // itself a little every frame.
    Panel::bottom("editor-footer")
        .frame(Frame::NONE)
        .show_separator_line(false)
        .resizable(false)
        .show(ui, |ui| {
            row_frame(Color32::TRANSPARENT).show(ui, |ui| {
                ui.horizontal(|ui| buttons(ui, state, selected, requests));
            });
            row_frame(Color32::TRANSPARENT).show(ui, |ui| {
                ui.horizontal(|ui| tone(ui, state));
            });
        });
    CentralPanel::default().frame(Frame::NONE).show(ui, |ui| match state.mode {
        Mode::Parametric => band_list(ui, state, selected, requests),
        Mode::Graphic => {
            Frame::new().inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                graphic::sliders(ui, &mut state.graphic);
            });
        }
    });
}

fn buttons(ui: &mut Ui, state: &mut EqState, selected: &mut Option<usize>, requests: &mut Requests) {
    if state.mode == Mode::Parametric {
        let full = state.bands.len() >= MAX_BANDS;
        let add = ui
            .add_enabled(!full, Button::new("Add Band"))
            .on_disabled_hover_text(FULL_MESSAGE);
        if add.clicked() {
            state.add_band(selected);
        }
        let remove = ui
            .add_enabled(!state.bands.is_empty(), Button::new("Remove All"))
            .on_hover_text("Remove every band. Undo with ⌘Z.");
        if remove.clicked() {
            state.bands.clear();
            *selected = None;
            requests.undoable = Some("All bands removed".into());
        }
    }
    let (hint, done) = match state.mode {
        Mode::Parametric => ("Restore the default flat bands and preamp", "Bands and preamp reset"),
        Mode::Graphic => ("Put every slider and the preamp at 0", "Sliders and preamp reset"),
    };
    if ui
        .add_enabled(state.can_reset() || state.tone_set(), Button::new("Reset"))
        .on_hover_text(format!("{hint}. Undo with ⌘Z."))
        .clicked()
    {
        if state.can_reset() {
            state.reset();
            *selected = None;
            requests.undoable = Some(done.into());
        } else {
            requests.notice = Some("Bass, treble and balance reset by double-clicking their sliders".into());
        }
    }
    ui.add_space(8.0);
    ui.checkbox(&mut state.clip_protection, "Clip Protection")
        .on_hover_text("Turns the output down for a moment when boosts would make it clip");
}

/// Bass and treble shelves and the left/right balance, shared by both modes.
fn tone(ui: &mut Ui, state: &mut EqState) {
    let p = &theme::palette(ui);
    ui.spacing_mut().slider_width = TONE_SLIDER_WIDTH;
    let caption = |ui: &mut Ui, text: &str| {
        ui.label(RichText::new(text).color(p.secondary));
    };
    for (name, value, hint) in [
        ("Bass", &mut state.bass_db, "Low shelf at 100 Hz. Double-click for 0 dB."),
        ("Treble", &mut state.treble_db, "High shelf at 10 kHz. Double-click for 0 dB."),
    ] {
        caption(ui, name);
        let slider = ui
            .add(
                Slider::new(value, -SLIDER_MAX_DB..=SLIDER_MAX_DB)
                    .step_by(0.5)
                    .custom_formatter(|v, _| format_gain(v as f32))
                    .custom_parser(parse_gain),
            )
            .on_hover_text(hint);
        if slider.double_clicked() {
            *value = 0.0;
        }
        ui.add_space(12.0);
    }
    caption(ui, "Balance");
    let slider = ui
        .add(
            Slider::new(&mut state.balance, -1.0..=1.0)
                .step_by(0.01)
                .custom_formatter(|v, _| format_balance(v as f32))
                .custom_parser(parse_balance),
        )
        .on_hover_text("Turns the other side down. Double-click to centre.");
    if slider.double_clicked() {
        state.balance = 0.0;
    }
    state.balance = snap_balance(state.balance);
}

/// "L 30", "R 30", "Centre", or a number from -100 (left) to 100 (right).
fn parse_balance(text: &str) -> Option<f64> {
    let text = text.trim().to_lowercase();
    if text.starts_with("c") {
        return Some(0.0);
    }
    let (sign, rest) = match text.strip_prefix('l') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('r').unwrap_or(&text)),
    };
    parse_gain(rest).map(|v| sign * v / 100.0)
}

/// One editable row per band under a header that names the columns.
fn band_list(ui: &mut Ui, state: &mut EqState, selected: &mut Option<usize>, requests: &mut Requests) {
    let p = &theme::palette(ui);
    let mut delete = None;
    let note = Cell::new(None);

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

    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        if state.bands.is_empty() {
            row_frame(Color32::TRANSPARENT).show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "No bands yet. Double-click the curve or use Add Band. Each band \
                         shapes one part of the sound.",
                    )
                    .color(p.secondary),
                );
            });
        }
        for (index, band) in state.bands.iter_mut().enumerate() {
            let fill = if *selected == Some(index) {
                p.row_selected
            } else {
                Color32::TRANSPARENT
            };
            let row = row_frame(fill)
                .show(ui, |ui| ui.horizontal(|ui| band_row(ui, index, band, p, &note)).inner)
                .inner;
            if row.touched {
                *selected = Some(index);
            }
            if row.delete {
                delete = Some(index);
            }
        }
    });

    if let Some(index) = delete {
        state.bands.remove(index);
        *selected = None;
        requests.undoable = Some(format!("Band {} deleted", index + 1));
    }
    if let Some(message) = note.get() {
        requests.notice = Some(message.into());
    }
}

struct RowResult {
    /// The row was interacted with and should become the selection.
    touched: bool,
    delete: bool,
}

/// Parses typed text with `parse`, and leaves a message in `note` when the text is not a
/// number or falls outside `range`, which the field then clamps to.
fn checked<'a>(
    parse: fn(&str) -> Option<f64>,
    range: std::ops::RangeInclusive<f64>,
    limited: &'static str,
    note: &'a Cell<Option<&'static str>>,
) -> impl Fn(&str) -> Option<f64> + 'a {
    move |text| match parse(text) {
        None => {
            note.set(Some("Enter a number."));
            None
        }
        Some(value) => {
            if !range.contains(&value) {
                note.set(Some(limited));
            }
            Some(value)
        }
    }
}

fn band_row(
    ui: &mut Ui,
    index: usize,
    band: &mut Band,
    p: &Palette,
    note: &Cell<Option<&'static str>>,
) -> RowResult {
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
            .custom_formatter(|v, _| {
                if v < 100.0 { format!("{v:.1}") } else { format!("{v:.0}") }
            })
            .custom_parser(checked(
                parse_freq,
                MIN_FREQ as f64..=MAX_FREQ as f64,
                "Frequency limited to 20 Hz to 20 kHz.",
                note,
            ))
            .update_while_editing(false)
            .suffix(" Hz"),
    ));
    ui.add_enabled_ui(band.kind.has_gain(), |ui| {
        let gain = ui.add_sized(
            [VALUE_WIDTH, ROW_HEIGHT],
            DragValue::new(&mut band.gain_db)
                .range(-MAX_GAIN_DB..=MAX_GAIN_DB)
                .speed(0.1)
                .fixed_decimals(1)
                .custom_parser(checked(
                    parse_gain,
                    -MAX_GAIN_DB as f64..=MAX_GAIN_DB as f64,
                    "Gain limited to \u{2212}24 dB to +24 dB.",
                    note,
                ))
                .update_while_editing(false)
                .suffix(" dB"),
        );
        track(&gain.on_disabled_hover_text("Pass filters have no gain"));
    });
    track(&ui.add_sized(
        [VALUE_WIDTH, ROW_HEIGHT],
        DragValue::new(&mut band.q)
            .range(MIN_Q..=MAX_Q)
            .speed(q_speed)
            .fixed_decimals(2)
            .custom_parser(checked(
                parse_gain,
                MIN_Q as f64..=MAX_Q as f64,
                "Q limited to 0.1 to 18.",
                note,
            ))
            .update_while_editing(false),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_reads_sides_and_numbers() {
        assert_eq!(parse_balance("L 30"), Some(-0.3));
        assert_eq!(parse_balance("r30"), Some(0.3));
        assert_eq!(parse_balance("Centre"), Some(0.0));
        assert_eq!(parse_balance("-50"), Some(-0.5));
        assert_eq!(parse_balance("left"), None);
    }

    #[test]
    fn typed_values_out_of_range_leave_a_message() {
        let note = Cell::new(None);
        let parse = checked(parse_freq, 20.0..=20_000.0, "limited", &note);
        assert_eq!(parse("1.5k"), Some(1_500.0));
        assert_eq!(note.get(), None);
        assert_eq!(parse("30k"), Some(30_000.0));
        assert_eq!(note.get(), Some("limited"));
        assert_eq!(parse("loud"), None);
        assert_eq!(note.get(), Some("Enter a number."));
    }
}
