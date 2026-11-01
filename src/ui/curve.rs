use super::theme::{self, CAPTION, Palette};
use super::{FULL_MESSAGE, OVER_THRESHOLD_DB, Requests, format_freq, format_gain, snap_freq, snap_gain};
use crate::dsp::response;
use crate::state::{
    Band, EqState, FilterKind, MAX_BANDS, MAX_FREQ, MAX_GAIN_DB, MAX_Q, MIN_FREQ, MIN_Q, Mode,
    SLIDER_MAX_DB,
};
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Mesh, Modifiers, Painter, Pos2, Rect, Sense,
    Shape, Stroke, Ui, pos2, vec2,
};

/// Room the graph keeps above and below the largest gain it shows.
const DB_MARGIN: f32 = 3.0;
/// The graphic view zooms out past this and back in under the next, so it does not
/// flicker between the two scales.
const ZOOM_OUT_ABOVE: f32 = 11.0;
const ZOOM_IN_BELOW: f32 = 9.0;
const NODE_RADIUS: f32 = 6.0;
/// Pointer target around a node, above the 20 pt macOS minimum.
const NODE_HIT: f32 = 24.0;
const CURVE_POINTS: usize = 256;
/// One keyboard nudge: a twelfth of an octave, half a dB, or this factor of Q.
const NUDGE_Q: f32 = 1.12;
const GRID_FREQS: [(f32, &str); 10] = [
    (20.0, "20"),
    (50.0, "50"),
    (100.0, "100"),
    (200.0, "200"),
    (500.0, "500"),
    (1_000.0, "1k"),
    (2_000.0, "2k"),
    (5_000.0, "5k"),
    (10_000.0, "10k"),
    (20_000.0, "20k"),
];

/// Maps between (frequency, dB) and screen positions inside the plot area.
#[derive(Clone, Copy)]
struct Axes {
    rect: Rect,
    /// Half the vertical span in dB.
    span: f32,
    /// Largest gain a drag can set.
    limit: f32,
}

impl Axes {
    fn x(&self, freq: f32) -> f32 {
        let t = (freq / MIN_FREQ).ln() / (MAX_FREQ / MIN_FREQ).ln();
        self.rect.left() + t * self.rect.width()
    }

    fn freq(&self, x: f32) -> f32 {
        let t = ((x - self.rect.left()) / self.rect.width()).clamp(0.0, 1.0);
        MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t)
    }

    fn y(&self, db: f32) -> f32 {
        self.rect.center().y - db / self.span * self.rect.height() / 2.0
    }

    fn db(&self, y: f32) -> f32 {
        ((self.rect.center().y - y) / (self.rect.height() / 2.0) * self.span)
            .clamp(-self.limit, self.limit)
    }

    fn node(&self, band: &Band) -> Pos2 {
        let db = if band.kind.has_gain() { band.gain_db } else { 0.0 };
        pos2(self.x(band.freq), self.y(db))
    }
}

/// A translucent fill between the curve and the 0 dB line.
fn fill(points: &[Pos2], zero_y: f32, color: Color32) -> Shape {
    let mut mesh = Mesh::default();
    for point in points {
        mesh.colored_vertex(*point, color);
        mesh.colored_vertex(pos2(point.x, zero_y), color);
    }
    for i in 0..points.len().saturating_sub(1) as u32 {
        let v = i * 2;
        mesh.add_triangle(v, v + 1, v + 2);
        mesh.add_triangle(v + 1, v + 2, v + 3);
    }
    Shape::mesh(mesh)
}

/// The largest gain the graph shows. Parametric bands reach ±24 dB; graphic sliders stop
/// at ±12, so that view zooms in unless bass or treble push the curve further.
fn view_limit(ui: &Ui, state: &EqState, peak_abs: f32) -> f32 {
    if state.mode == Mode::Parametric {
        return MAX_GAIN_DB;
    }
    let id = Id::new("graphic-zoom");
    let mut limit = ui.data(|d| d.get_temp(id)).unwrap_or(SLIDER_MAX_DB);
    // The scale holds still under a drag, so the point does not run from the pointer.
    if !ui.input(|i| i.pointer.any_down()) {
        if limit < MAX_GAIN_DB && peak_abs > ZOOM_OUT_ABOVE {
            limit = MAX_GAIN_DB;
        } else if limit > SLIDER_MAX_DB && peak_abs < ZOOM_IN_BELOW {
            limit = SLIDER_MAX_DB;
        }
        ui.data_mut(|d| d.insert_temp(id, limit));
    }
    ui.ctx().animate_value_with_time(id.with("animated"), limit, 0.3)
}

/// The shape for a filter type: a circle for those with gain, a diamond for pass filters,
/// with a tail on the side a shelf or pass filter acts on.
fn glyph(painter: &Painter, center: Pos2, band: &Band, radius: f32, color: Color32, ring: Color32) {
    let tail = match band.kind {
        FilterKind::LowShelf | FilterKind::HighPass => Some(-1.0),
        FilterKind::HighShelf | FilterKind::LowPass => Some(1.0),
        FilterKind::Peak => None,
    };
    if let Some(side) = tail {
        let start = center + vec2(side * radius, 0.0);
        let end = center + vec2(side * (radius + 9.0), 0.0);
        painter.line_segment([start, end], Stroke::new(4.0, ring));
        painter.line_segment([start, end], Stroke::new(2.0, color));
    }
    let shape = |r: f32| -> Vec<Pos2> {
        vec![
            center + vec2(0.0, -r),
            center + vec2(r, 0.0),
            center + vec2(0.0, r),
            center + vec2(-r, 0.0),
        ]
    };
    if band.kind.has_gain() {
        painter.circle_filled(center, radius + 2.0, ring);
        if band.enabled {
            painter.circle_filled(center, radius, color);
        } else {
            painter.circle_stroke(center, radius - 0.75, Stroke::new(1.5, color));
        }
    } else {
        let r = radius * 1.25;
        painter.add(Shape::convex_polygon(shape(r + 2.5), ring, Stroke::NONE));
        if band.enabled {
            painter.add(Shape::convex_polygon(shape(r), color, Stroke::NONE));
        } else {
            painter.add(Shape::closed_line(shape(r - 1.0), Stroke::new(1.5, color)));
        }
    }
}

/// A label in a filled, rounded box, kept inside `bounds`.
fn chip(painter: &Painter, anchor: Pos2, above: bool, text: String, bounds: Rect, p: &Palette) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(theme::SMALL), p.plot);
    let size = galley.size() + vec2(12.0, 6.0);
    let mut rect = Rect::from_center_size(anchor, size);
    rect = rect.translate(vec2(0.0, if above { -(size.y / 2.0 + 16.0) } else { size.y / 2.0 + 16.0 }));
    let dx = (bounds.left() + 2.0 - rect.left()).max(0.0) + (bounds.right() - 2.0 - rect.right()).min(0.0);
    rect = rect.translate(vec2(dx, 0.0));
    painter.rect_filled(rect, CornerRadius::same(6), p.label);
    painter.galley(rect.min + vec2(6.0, 3.0), galley, p.plot);
}

/// Draws the frequency response and handles editing it. In parametric mode, drag a node to
/// set frequency and gain, scroll over it to change Q, right-click for filter type, and
/// double-click empty space to add a band. In graphic mode, drag a slider's point up or
/// down. The arrow keys nudge the selected band.
pub fn show(
    ui: &mut Ui,
    state: &mut EqState,
    selected: &mut Option<usize>,
    sample_rate: f64,
    keys: bool,
    requests: &mut Requests,
) {
    let p = &theme::palette(ui);
    let size = vec2(ui.available_width(), ui.available_height().max(160.0));
    let (outer, background) = ui.allocate_exact_size(size, Sense::click());
    let plot_rect = Rect::from_min_max(outer.min + vec2(30.0, 0.0), outer.max - vec2(0.0, 20.0));

    let xs: Vec<f32> = (0..CURVE_POINTS)
        .map(|i| plot_rect.left() + plot_rect.width() * i as f32 / (CURVE_POINTS - 1) as f32)
        .collect();
    let probe = Axes { rect: plot_rect, span: 1.0, limit: 1.0 };
    let freqs: Vec<f32> = xs.iter().map(|&x| probe.freq(x)).collect();
    let total_db = response::total_db(state, &freqs, sample_rate);
    let peak_abs = total_db.iter().fold(0f32, |m, v| m.max(v.abs()));

    let limit = view_limit(ui, state, peak_abs);
    let axes = Axes {
        rect: plot_rect,
        span: limit + DB_MARGIN,
        limit: if state.mode == Mode::Graphic { SLIDER_MAX_DB } else { MAX_GAIN_DB },
    };
    let painter = ui.painter_at(outer.expand(8.0));
    let hairline = Stroke::new(1.0, p.hairline);
    let font = FontId::proportional(CAPTION);
    let accent = if state.bypass { p.secondary } else { p.accent };

    painter.rect_filled(axes.rect, 8.0, p.plot);
    for (i, (freq, label)) in GRID_FREQS.into_iter().enumerate() {
        let x = axes.x(freq);
        let edge = i == 0 || i == GRID_FREQS.len() - 1;
        if !edge {
            painter.vline(x, axes.rect.y_range(), hairline);
        }
        let align = match i {
            0 => Align2::LEFT_TOP,
            _ if edge => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        painter.text(pos2(x, axes.rect.bottom() + 5.0), align, label, font.clone(), p.secondary);
    }
    // Lines every 6 dB at ±24, every 3 dB at ±12, labelled every other line.
    let step = if limit <= SLIDER_MAX_DB + 0.5 { 3.0 } else { 6.0 };
    let lines = (limit / step).round() as i32;
    for line in -lines..=lines {
        let db = line as f32 * step;
        let y = axes.y(db);
        let stroke = if line == 0 { Stroke::new(1.0, p.zero_line) } else { hairline };
        painter.hline(axes.rect.x_range(), y, stroke);
        if line % 2 == 0 {
            painter.text(
                pos2(axes.rect.left() - 8.0, y),
                Align2::RIGHT_CENTER,
                if line == 0 { "0".into() } else { format!("{db:+.0}") },
                font.clone(),
                p.secondary,
            );
        }
    }

    let plot = ui.painter_at(axes.rect);
    let points = |dbs: &[f32]| -> Vec<Pos2> {
        xs.iter().zip(dbs).map(|(&x, &db)| pos2(x, axes.y(db))).collect()
    };
    let total = points(&total_db);
    plot.add(fill(&total, axes.y(0.0), accent.gamma_multiply(0.14)));
    if state.mode == Mode::Parametric
        && let Some(band) = selected.and_then(|i| state.bands.get(i)).filter(|b| b.enabled)
    {
        let dbs: Vec<f32> = freqs.iter().map(|&f| response::band_db(band, f, sample_rate)).collect();
        plot.add(Shape::line(points(&dbs), Stroke::new(1.0, accent.gamma_multiply(0.55))));
    }
    let width = if state.bypass { 1.5 } else { 2.0 };
    plot.add(Shape::line(total.clone(), Stroke::new(width, accent)));

    // Where the curve goes past full scale with this preamp, it would clip.
    let ceiling = -state.preamp_db;
    let over = total_db.iter().any(|&db| db > ceiling + OVER_THRESHOLD_DB);
    if over && !state.bypass && !state.clip_protection {
        let y = axes.y(ceiling);
        plot.add(Shape::dashed_line(
            &[pos2(axes.rect.left(), y), pos2(axes.rect.right(), y)],
            Stroke::new(1.0, p.danger.gamma_multiply(0.7)),
            5.0,
            4.0,
        ));
        let mut run: Vec<Pos2> = Vec::new();
        for (point, &db) in total.iter().zip(&total_db) {
            if db > ceiling + OVER_THRESHOLD_DB {
                run.push(*point);
            } else if !run.is_empty() {
                plot.add(Shape::line(std::mem::take(&mut run), Stroke::new(2.0, p.danger)));
            }
        }
        if !run.is_empty() {
            plot.add(Shape::line(run, Stroke::new(2.0, p.danger)));
        }
    }
    if state.bypass {
        plot.text(
            axes.rect.left_top() + vec2(10.0, 8.0),
            Align2::LEFT_TOP,
            "Bypassed",
            FontId::proportional(theme::SMALL),
            p.secondary,
        );
    }

    match state.mode {
        Mode::Parametric => {
            if keys {
                nudge(ui, state, *selected);
            }
            bands(ui, &painter, &axes, state, selected, &background, requests, p)
        }
        Mode::Graphic => sliders(ui, &painter, &axes, state, p),
    }
}

/// The parametric band nodes, and everything that edits them on the graph.
#[allow(clippy::too_many_arguments)]
fn bands(
    ui: &mut Ui,
    painter: &Painter,
    axes: &Axes,
    state: &mut EqState,
    selected: &mut Option<usize>,
    background: &eframe::egui::Response,
    requests: &mut Requests,
    p: &Palette,
) {
    let accent = if state.bypass { p.secondary } else { p.accent };
    let font = FontId::proportional(CAPTION);

    if background.double_clicked()
        && let Some(pos) = background.interact_pointer_pos()
        && axes.rect.contains(pos)
    {
        if state.bands.len() < MAX_BANDS {
            let mut band = Band::new(FilterKind::Peak, snap_freq(axes.freq(pos.x)));
            band.gain_db = snap_gain(axes.db(pos.y));
            band.q = 1.0;
            band.clamp();
            state.bands.push(band);
            *selected = Some(state.bands.len() - 1);
        } else {
            requests.notice = Some(FULL_MESSAGE.into());
        }
    } else if background.clicked() {
        *selected = None;
    }

    let mut delete = None;
    let mut dragging = None;
    for (index, band) in state.bands.iter_mut().enumerate() {
        let hit = Rect::from_center_size(axes.node(band), vec2(NODE_HIT, NODE_HIT));
        let node = ui.interact(hit, ui.id().with(("eq-node", index)), Sense::click_and_drag());

        if node.clicked() || node.drag_started() {
            *selected = Some(index);
        }
        if node.dragged()
            && let Some(pos) = node.interact_pointer_pos()
        {
            band.freq = snap_freq(axes.freq(pos.x)).clamp(MIN_FREQ, MAX_FREQ);
            if band.kind.has_gain() {
                band.gain_db = snap_gain(axes.db(pos.y));
            }
            dragging = Some(index);
        }
        if node.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 || zoom != 1.0 {
                band.q = (band.q * (scroll * 0.01).exp() * zoom).clamp(MIN_Q, MAX_Q);
            }
        }
        node.context_menu(|ui| {
            for kind in FilterKind::ALL {
                if ui.selectable_label(band.kind == kind, kind.label()).clicked() {
                    band.kind = kind;
                    ui.close();
                }
            }
            ui.separator();
            if ui.checkbox(&mut band.enabled, "Enabled").clicked() {
                ui.close();
            }
            let invertible = band.kind.has_gain() && band.gain_db != 0.0;
            if ui.add_enabled(invertible, eframe::egui::Button::new("Invert Gain")).clicked() {
                band.gain_db = -band.gain_db;
                ui.close();
            }
            if ui.button("Delete Band").clicked() {
                delete = Some(index);
                ui.close();
            }
        });
        if dragging != Some(index) {
            node.on_hover_text(describe(index, band));
        }
    }

    // Guides from the dragged node to the axes it is being read against.
    if let Some(band) = dragging.and_then(|i| state.bands.get(i)) {
        let center = axes.node(band);
        let guide = Stroke::new(1.0, accent.gamma_multiply(0.4));
        painter.vline(center.x, center.y..=axes.rect.bottom(), guide);
        if band.kind.has_gain() {
            painter.hline(axes.rect.left()..=center.x, center.y, guide);
        }
    }

    // Nodes, with the selected one drawn last so it sits on top.
    let mut order: Vec<usize> = (0..state.bands.len()).filter(|&i| Some(i) != *selected).collect();
    order.extend(selected.filter(|&i| i < state.bands.len()));
    for &index in &order {
        let band = &state.bands[index];
        let center = axes.node(band);
        let active = *selected == Some(index) || dragging == Some(index);
        if active {
            painter.circle_filled(center, NODE_RADIUS + 6.0, accent.gamma_multiply(0.25));
        }
        let radius = if dragging == Some(index) { NODE_RADIUS + 1.5 } else { NODE_RADIUS };
        // A plot-colored ring keeps the node distinct from the curve it sits on.
        glyph(painter, center, band, radius, accent, p.plot);
    }

    // Band numbers sit above their node, or below when there is no room. A number that
    // would cover another node or number is left out, except the selected band's.
    let mut taken: Vec<Rect> = Vec::new();
    for &index in order.iter().rev() {
        let center = axes.node(&state.bands[index]);
        let above = center.y - axes.rect.top() > 26.0;
        let anchor = center + vec2(0.0, if above { -12.0 } else { 12.0 });
        let text = (index + 1).to_string();
        let rect = Rect::from_center_size(
            anchor + vec2(0.0, if above { -6.0 } else { 6.0 }),
            vec2(7.0 * text.len() as f32 + 2.0, 12.0),
        );
        let covers_node = state.bands.iter().enumerate().any(|(other, b)| {
            other != index && rect.expand(NODE_RADIUS).contains(axes.node(b))
        });
        let is_selected = *selected == Some(index);
        if !is_selected && (covers_node || taken.iter().any(|t| t.intersects(rect))) {
            continue;
        }
        taken.push(rect);
        painter.text(
            anchor,
            if above { Align2::CENTER_BOTTOM } else { Align2::CENTER_TOP },
            text,
            font.clone(),
            if is_selected { p.label } else { p.secondary },
        );
    }

    if let Some(index) = dragging
        && let Some(band) = state.bands.get(index)
    {
        let center = axes.node(band);
        let above = center.y - axes.rect.top() > 60.0;
        chip(painter, center, above, describe(index, band), axes.rect, p);
    }

    if let Some(index) = delete {
        state.bands.remove(index);
        *selected = None;
        requests.undoable = Some(format!("Band {} deleted", index + 1));
    }
}

/// "3  1.25 kHz  +3.5 dB  Q 1.00", leaving out the gain of pass filters.
fn describe(index: usize, band: &Band) -> String {
    let gain = if band.kind.has_gain() {
        format!("{}  ", format_gain(band.gain_db))
    } else {
        String::new()
    };
    format!(
        "{}  {}  {}  {gain}Q {:.2}",
        index + 1,
        band.kind.label(),
        format_freq(band.freq),
        band.q
    )
}

/// Arrow keys move the selected band: left and right by a twelfth of an octave, up and
/// down by half a dB, and with Option up and down change Q.
fn nudge(ui: &Ui, state: &mut EqState, selected: Option<usize>) {
    let Some(band) = selected.and_then(|i| state.bands.get_mut(i)) else {
        return;
    };
    // Keys belong to a text field or other control while one has focus, and to a menu
    // while one is open.
    if ui.ctx().memory(|m| m.focused().is_some()) || ui.ctx().any_popup_open() {
        return;
    }
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
            band.q = ((band.q * NUDGE_Q * 100.0).round() / 100.0).clamp(MIN_Q, MAX_Q);
        }
        if i.consume_key(Modifiers::ALT, Key::ArrowDown) {
            band.q = ((band.q / NUDGE_Q * 100.0).round() / 100.0).clamp(MIN_Q, MAX_Q);
        }
        let mut octaves = 0.0;
        if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
            octaves += 1.0 / 12.0;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            octaves -= 1.0 / 12.0;
        }
        if octaves != 0.0 {
            band.freq = snap_freq(band.freq * 2f32.powf(octaves)).clamp(MIN_FREQ, MAX_FREQ);
        }
        if band.kind.has_gain() {
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                band.gain_db = snap_gain(band.gain_db + 0.5).min(MAX_GAIN_DB);
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                band.gain_db = snap_gain(band.gain_db - 0.5).max(-MAX_GAIN_DB);
            }
        }
    });
}

/// A point for each graphic slider, which can be dragged up and down like the slider.
fn sliders(ui: &mut Ui, painter: &Painter, axes: &Axes, state: &mut EqState, p: &Palette) {
    let accent = if state.bypass { p.secondary } else { p.accent };
    let centres = state.graphic.centres();
    let radius = if centres.len() > 15 { 3.0 } else { 4.0 };
    let spacing = axes.rect.width() / centres.len().max(1) as f32;
    let hit = NODE_HIT.min(spacing.max(8.0));
    for (index, (gain, &freq)) in state.graphic.gains_db.iter_mut().zip(centres).enumerate() {
        let center = pos2(axes.x(freq), axes.y(*gain));
        let rect = Rect::from_center_size(center, vec2(hit, NODE_HIT));
        let point = ui.interact(rect, ui.id().with(("graphic-point", index)), Sense::click_and_drag());
        if point.dragged()
            && let Some(pos) = point.interact_pointer_pos()
        {
            *gain = ((axes.db(pos.y) * 2.0).round() / 2.0).clamp(-SLIDER_MAX_DB, SLIDER_MAX_DB);
        }
        if point.double_clicked() {
            *gain = 0.0;
        }
        let center = pos2(axes.x(freq), axes.y(*gain));
        let active = point.dragged() || point.hovered();
        if point.dragged() {
            painter.vline(center.x, axes.rect.y_range(), Stroke::new(1.0, accent.gamma_multiply(0.4)));
        }
        painter.circle_filled(center, radius + 2.0, p.plot);
        painter.circle_filled(center, if active { radius + 2.0 } else { radius }, accent);
        if active {
            point.on_hover_text(format!("{}   {}", format_freq(freq), format_gain(*gain)));
        }
    }
}
