use super::band_color;
use crate::dsp::response;
use crate::state::{
    Band, EqState, FilterKind, MAX_BANDS, MAX_FREQ, MAX_GAIN_DB, MAX_Q, MIN_FREQ, MIN_Q,
};
use eframe::egui::{
    Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};

/// Half the vertical span of the graph in dB, a little more than the gain limit.
const DB_SPAN: f32 = MAX_GAIN_DB + 3.0;
const NODE_RADIUS: f32 = 8.0;
const CURVE_POINTS: usize = 256;
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
        self.rect.center().y - db / DB_SPAN * self.rect.height() / 2.0
    }

    fn db(&self, y: f32) -> f32 {
        ((self.rect.center().y - y) / (self.rect.height() / 2.0) * DB_SPAN)
            .clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
    }

    fn node(&self, band: &Band) -> Pos2 {
        let db = if band.kind.has_gain() { band.gain_db } else { 0.0 };
        pos2(self.x(band.freq), self.y(db))
    }
}

/// Draws the frequency response and handles dragging band nodes. Drag a node to set
/// frequency and gain, scroll over it to change Q, right-click for filter type, and
/// double-click empty space to add a band.
pub fn show(ui: &mut Ui, state: &mut EqState, selected: &mut Option<usize>, sample_rate: f64) {
    let size = vec2(ui.available_width(), ui.available_height().max(160.0));
    let (outer, background) = ui.allocate_exact_size(size, Sense::click());
    let axes = Axes {
        rect: Rect::from_min_max(outer.min + vec2(34.0, 8.0), outer.max - vec2(10.0, 20.0)),
    };
    let painter = ui.painter_at(outer);
    let visuals = ui.visuals();
    let grid = Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.35));
    let label_color = visuals.weak_text_color();
    let font = FontId::proportional(10.0);
    let accent = if state.bypass {
        visuals.weak_text_color()
    } else {
        visuals.selection.stroke.color
    };

    painter.rect_filled(axes.rect, 4.0, visuals.extreme_bg_color);
    for (freq, label) in GRID_FREQS {
        let x = axes.x(freq);
        painter.vline(x, axes.rect.y_range(), grid);
        painter.text(
            pos2(x, axes.rect.bottom() + 4.0),
            Align2::CENTER_TOP,
            label,
            font.clone(),
            label_color,
        );
    }
    for step in -4..=4 {
        let db = step as f32 * 6.0;
        let y = axes.y(db);
        let stroke = if step == 0 {
            Stroke::new(1.0, visuals.weak_text_color())
        } else {
            grid
        };
        painter.hline(axes.rect.x_range(), y, stroke);
        painter.text(
            pos2(axes.rect.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{db:+.0}"),
            font.clone(),
            label_color,
        );
    }
    painter.rect_stroke(axes.rect, 4.0, grid, StrokeKind::Inside);

    let plot = ui.painter_at(axes.rect);
    let xs: Vec<f32> = (0..CURVE_POINTS)
        .map(|i| axes.rect.left() + axes.rect.width() * i as f32 / (CURVE_POINTS - 1) as f32)
        .collect();
    let freqs: Vec<f32> = xs.iter().map(|&x| axes.freq(x)).collect();
    let line = |dbs: Vec<f32>, stroke: Stroke| {
        let points = xs.iter().zip(dbs).map(|(&x, db)| pos2(x, axes.y(db))).collect();
        Shape::line(points, stroke)
    };

    if let Some(band) = selected.and_then(|i| state.bands.get(i)).filter(|b| b.enabled) {
        let dbs = freqs.iter().map(|&f| response::band_db(band, f, sample_rate)).collect();
        let color = band_color(selected.unwrap()).gamma_multiply(0.6);
        plot.add(line(dbs, Stroke::new(1.0, color)));
    }
    let total = response::total_db(state, &freqs, sample_rate);
    plot.add(line(total, Stroke::new(2.0, accent)));

    if background.double_clicked()
        && state.bands.len() < MAX_BANDS
        && let Some(pos) = background.interact_pointer_pos()
        && axes.rect.contains(pos)
    {
        let mut band = Band::new(FilterKind::Peak, axes.freq(pos.x));
        band.gain_db = axes.db(pos.y);
        band.q = 1.0;
        state.bands.push(band);
        *selected = Some(state.bands.len() - 1);
    } else if background.clicked() {
        *selected = None;
    }

    let mut delete = None;
    for (index, band) in state.bands.iter_mut().enumerate() {
        let center = axes.node(band);
        let hit = Rect::from_center_size(center, vec2(NODE_RADIUS * 2.8, NODE_RADIUS * 2.8));
        let node = ui.interact(hit, ui.id().with(("eq-node", index)), Sense::click_and_drag());

        if node.clicked() || node.drag_started() {
            *selected = Some(index);
        }
        if node.dragged()
            && let Some(pos) = node.interact_pointer_pos()
        {
            band.freq = axes.freq(pos.x);
            if band.kind.has_gain() {
                band.gain_db = axes.db(pos.y);
            }
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
            if ui.button("Delete band").clicked() {
                delete = Some(index);
                ui.close();
            }
        });
        let node = node.on_hover_text(format!(
            "{} · {:.0} Hz · {:+.1} dB · Q {:.2}",
            band.kind.label(),
            band.freq,
            band.gain_db,
            band.q
        ));

        let color = band_color(index);
        let center = axes.node(band);
        let active = *selected == Some(index) || node.hovered() || node.dragged();
        if band.enabled && !state.bypass {
            painter.circle_filled(center, NODE_RADIUS, color);
        } else {
            painter.circle_filled(center, NODE_RADIUS, visuals.extreme_bg_color);
            painter.circle_stroke(center, NODE_RADIUS, Stroke::new(1.5, color));
        }
        if active {
            painter.circle_stroke(center, NODE_RADIUS + 3.0, Stroke::new(1.5, color));
        }
        painter.text(
            center,
            Align2::CENTER_CENTER,
            (index + 1).to_string(),
            FontId::proportional(10.0),
            if band.enabled && !state.bypass {
                Color32::BLACK
            } else {
                color
            },
        );
    }
    if let Some(index) = delete {
        state.bands.remove(index);
        *selected = None;
    }
}
