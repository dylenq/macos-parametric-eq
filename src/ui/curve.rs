use super::format_freq;
use super::theme::{self, CAPTION};
use crate::dsp::response;
use crate::state::{
    Band, EqState, FilterKind, MAX_BANDS, MAX_FREQ, MAX_GAIN_DB, MAX_Q, MIN_FREQ, MIN_Q,
};
use eframe::egui::{
    Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2,
};

/// Half the vertical span of the graph in dB, a little more than the gain limit.
const DB_SPAN: f32 = MAX_GAIN_DB + 3.0;
const NODE_RADIUS: f32 = 6.0;
/// Pointer target around a node, above the 20 pt macOS minimum.
const NODE_HIT: f32 = 24.0;
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

/// Draws the frequency response and handles dragging band nodes. Drag a node to set
/// frequency and gain, scroll over it to change Q, right-click for filter type, and
/// double-click empty space to add a band.
pub fn show(ui: &mut Ui, state: &mut EqState, selected: &mut Option<usize>, sample_rate: f64) {
    let p = &theme::palette(ui);
    let size = vec2(ui.available_width(), ui.available_height().max(160.0));
    let (outer, background) = ui.allocate_exact_size(size, Sense::click());
    let axes = Axes {
        rect: Rect::from_min_max(outer.min + vec2(30.0, 0.0), outer.max - vec2(0.0, 20.0)),
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
    for step in -4..=4 {
        let db = step as f32 * 6.0;
        let y = axes.y(db);
        let stroke = if step == 0 {
            Stroke::new(1.0, p.zero_line)
        } else {
            hairline
        };
        painter.hline(axes.rect.x_range(), y, stroke);
        // Label every other line to keep the axis quiet.
        if step % 2 == 0 {
            painter.text(
                pos2(axes.rect.left() - 8.0, y),
                Align2::RIGHT_CENTER,
                if step == 0 { "0".into() } else { format!("{db:+.0}") },
                font.clone(),
                p.secondary,
            );
        }
    }

    let plot = ui.painter_at(axes.rect);
    let xs: Vec<f32> = (0..CURVE_POINTS)
        .map(|i| axes.rect.left() + axes.rect.width() * i as f32 / (CURVE_POINTS - 1) as f32)
        .collect();
    let freqs: Vec<f32> = xs.iter().map(|&x| axes.freq(x)).collect();
    let points = |dbs: Vec<f32>| -> Vec<Pos2> {
        xs.iter().zip(dbs).map(|(&x, db)| pos2(x, axes.y(db))).collect()
    };

    let total = points(response::total_db(state, &freqs, sample_rate));
    plot.add(fill(&total, axes.y(0.0), accent.gamma_multiply(0.14)));
    if let Some(band) = selected.and_then(|i| state.bands.get(i)).filter(|b| b.enabled) {
        let dbs = freqs.iter().map(|&f| response::band_db(band, f, sample_rate)).collect();
        plot.add(Shape::line(points(dbs), Stroke::new(1.0, accent.gamma_multiply(0.55))));
    }
    plot.add(Shape::line(total, Stroke::new(2.0, accent)));

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
        let hit = Rect::from_center_size(axes.node(band), vec2(NODE_HIT, NODE_HIT));
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
            if ui.button("Delete Band").clicked() {
                delete = Some(index);
                ui.close();
            }
        });
        let gain = if band.kind.has_gain() {
            format!("{:+.1} dB · ", band.gain_db)
        } else {
            String::new()
        };
        let node = node.on_hover_text(format!(
            "{} · {} · {gain}Q {:.2}",
            band.kind.label(),
            format_freq(band.freq),
            band.q
        ));

        let center = axes.node(band);
        if *selected == Some(index) || node.hovered() || node.dragged() {
            painter.circle_filled(center, NODE_RADIUS + 6.0, accent.gamma_multiply(0.25));
        }
        // A plot-colored ring keeps the node distinct from the curve it sits on.
        painter.circle_filled(center, NODE_RADIUS + 2.0, p.plot);
        if band.enabled {
            painter.circle_filled(center, NODE_RADIUS, accent);
        } else {
            painter.circle_stroke(center, NODE_RADIUS - 0.75, Stroke::new(1.5, accent));
        }
        // The band number sits above the node, or below when there is no room.
        let above = center.y - axes.rect.top() > 26.0;
        painter.text(
            center + vec2(0.0, if above { -12.0 } else { 12.0 }),
            if above { Align2::CENTER_BOTTOM } else { Align2::CENTER_TOP },
            (index + 1).to_string(),
            font.clone(),
            if *selected == Some(index) { p.label } else { p.secondary },
        );
    }
    if let Some(index) = delete {
        state.bands.remove(index);
        *selected = None;
    }
}
