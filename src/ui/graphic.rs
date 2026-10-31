use super::theme::{self, CAPTION};
use super::{format_freq, format_gain, short_freq};
use crate::state::{Graphic, SLIDER_MAX_DB};
use eframe::egui::{
    Align, Align2, FontId, Label, Layout, RichText, ScrollArea, Sense, Slider, Ui, vec2,
};

/// Narrowest a slider column gets before the row scrolls instead.
const MIN_COLUMN: f32 = 30.0;
/// Columns this wide or wider show their gain above the slider.
const GAIN_LABEL_COLUMN: f32 = 44.0;
const SCALE_WIDTH: f32 = 30.0;
const LABEL_HEIGHT: f32 = 16.0;

/// One vertical slider per frequency, ±12 dB in half-dB steps. Double-click a slider to
/// put it back at 0.
pub fn sliders(ui: &mut Ui, graphic: &mut Graphic) {
    let p = &theme::palette(ui);
    let centres = graphic.centres();
    let count = centres.len().min(graphic.gains_db.len());
    let mut touched = None;

    // The readout sits above the sliders and says what the one in use is set to.
    let readout = ui.allocate_space(vec2(ui.available_width(), LABEL_HEIGHT)).1;

    let height = ui.available_height().max(96.0);
    let width = ui.available_width() - SCALE_WIDTH;
    let column = (width / count.max(1) as f32).max(MIN_COLUMN);
    let show_gain = column >= GAIN_LABEL_COLUMN;
    let track = height - LABEL_HEIGHT * if show_gain { 2.0 } else { 1.0 } - 8.0;

    ui.horizontal_top(|ui| {
        // dB scale, lined up with the slider tracks.
        let (scale, _) = ui.allocate_exact_size(vec2(SCALE_WIDTH, height), Sense::hover());
        let top = scale.top() + if show_gain { LABEL_HEIGHT } else { 0.0 } + 8.0;
        let font = FontId::proportional(CAPTION);
        for (fraction, text) in [(0.0, "+12"), (0.5, "0"), (1.0, "\u{2212}12")] {
            ui.painter().text(
                scale.right_top() + vec2(-6.0, top - scale.top() + fraction * track),
                Align2::RIGHT_CENTER,
                text,
                font.clone(),
                p.secondary,
            );
        }

        ScrollArea::horizontal()
            .id_salt(("graphic", count))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().slider_width = track.max(40.0);
                for (index, (gain, &freq)) in graphic.gains_db.iter_mut().zip(centres).enumerate() {
                    ui.allocate_ui_with_layout(
                        vec2(column, height),
                        Layout::top_down(Align::Center),
                        |ui| {
                            ui.set_width(column);
                            if show_gain {
                                ui.add_sized(
                                    [column, LABEL_HEIGHT],
                                    Label::new(
                                        RichText::new(format_gain(*gain).replace(" dB", ""))
                                            .size(CAPTION)
                                            .color(p.secondary),
                                    ),
                                );
                            }
                            ui.add_space(8.0);
                            let slider = ui.add(
                                Slider::new(gain, -SLIDER_MAX_DB..=SLIDER_MAX_DB)
                                    .vertical()
                                    .step_by(0.5)
                                    .show_value(false)
                                    .trailing_fill(false),
                            );
                            if slider.double_clicked() {
                                *gain = 0.0;
                            }
                            if slider.hovered() || slider.dragged() || slider.has_focus() {
                                touched = Some(index);
                            }
                            slider.on_hover_text(format!(
                                "{} · {}",
                                format_freq(freq),
                                format_gain(*gain)
                            ));
                            ui.add_sized(
                                [column, LABEL_HEIGHT],
                                Label::new(
                                    RichText::new(short_freq(freq)).size(CAPTION).color(p.secondary),
                                ),
                            );
                        },
                    );
                }
            });
    });

    let text = match touched {
        Some(index) => format!(
            "{}   {}",
            format_freq(centres[index]),
            format_gain(graphic.gains_db[index])
        ),
        None => "Drag a slider. Double-click to reset it.".into(),
    };
    ui.painter().text(
        readout.left_center() + vec2(SCALE_WIDTH, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(theme::SMALL),
        p.secondary,
    );
}
