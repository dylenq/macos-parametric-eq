use eframe::egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Style,
    TextStyle, Theme, Ui, style::HandleShape, vec2,
};
use objc2_app_kit::{NSColor, NSColorSpace};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// macOS text sizes: 13 pt body, 11 pt secondary, 10 pt as the smallest.
pub const BODY: f32 = 13.0;
pub const SMALL: f32 = 11.0;
pub const CAPTION: f32 = 10.0;

const SYSTEM_FONT: &str = "/System/Library/Fonts/SFNS.ttf";

/// Colors named by role, with a variant for each system appearance.
#[derive(Clone, Copy)]
pub struct Palette {
    pub window: Color32,
    /// The graph's background.
    pub plot: Color32,
    popover: Color32,
    pub control: Color32,
    control_hover: Color32,
    pub control_active: Color32,
    pub label: Color32,
    /// Supporting text: axis labels, column headers, status.
    pub secondary: Color32,
    pub hairline: Color32,
    /// The 0 dB reference line.
    pub zero_line: Color32,
    pub row_selected: Color32,
    /// The response curve, band nodes and "on" states. Follows the system accent color.
    pub accent: Color32,
    /// Fill behind white text, darker than `accent` to keep it readable.
    selection: Color32,
    /// Clipping.
    pub danger: Color32,
}

const DARK: Palette = Palette {
    window: Color32::from_rgb(0x1e, 0x1e, 0x1e),
    plot: Color32::from_rgb(0x17, 0x17, 0x17),
    popover: Color32::from_rgb(0x2a, 0x2a, 0x2a),
    control: Color32::from_rgb(0x33, 0x33, 0x33),
    control_hover: Color32::from_rgb(0x3d, 0x3d, 0x3d),
    control_active: Color32::from_rgb(0x4a, 0x4a, 0x4a),
    label: Color32::from_rgb(0xe8, 0xe8, 0xe8),
    secondary: Color32::from_rgb(0x9c, 0x9c, 0x9c),
    hairline: Color32::from_rgba_premultiplied(18, 18, 18, 18),
    zero_line: Color32::from_rgba_premultiplied(56, 56, 56, 56),
    row_selected: Color32::from_rgba_premultiplied(16, 16, 16, 16),
    accent: Color32::from_rgb(0x0a, 0x84, 0xff),
    selection: Color32::from_rgb(0x0b, 0x63, 0xd6),
    danger: Color32::from_rgb(0xff, 0x45, 0x3a),
};

const LIGHT: Palette = Palette {
    window: Color32::from_rgb(0xec, 0xec, 0xec),
    plot: Color32::from_rgb(0xff, 0xff, 0xff),
    popover: Color32::from_rgb(0xf6, 0xf6, 0xf6),
    control: Color32::from_rgb(0xdc, 0xdc, 0xdc),
    control_hover: Color32::from_rgb(0xd2, 0xd2, 0xd2),
    control_active: Color32::from_rgb(0xc4, 0xc4, 0xc4),
    label: Color32::from_rgb(0x1d, 0x1d, 0x1f),
    secondary: Color32::from_rgb(0x5f, 0x5f, 0x63),
    hairline: Color32::from_rgba_premultiplied(0, 0, 0, 24),
    zero_line: Color32::from_rgba_premultiplied(0, 0, 0, 72),
    row_selected: Color32::from_rgba_premultiplied(0, 0, 0, 18),
    accent: Color32::from_rgb(0x00, 0x7a, 0xff),
    selection: Color32::from_rgb(0x0b, 0x63, 0xd6),
    danger: Color32::from_rgb(0xd7, 0x00, 0x15),
};

/// The accent color chosen in System Settings as 0xFFRRGGBB, or 0 before it is read.
static SYSTEM_ACCENT: AtomicU32 = AtomicU32::new(0);

fn read_system_accent() -> Option<Color32> {
    let color = NSColor::controlAccentColor();
    let srgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(Color32::from_rgb(
        channel(srgb.redComponent()),
        channel(srgb.greenComponent()),
        channel(srgb.blueComponent()),
    ))
}

fn luminance(color: Color32) -> f32 {
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

/// Darkens `color` until white text on it reaches 4.5:1.
fn readable_fill(mut color: Color32) -> Color32 {
    while 1.05 / (luminance(color) + 0.05) < 4.5 {
        let darker = |c: u8| (c as f32 * 0.92) as u8;
        color = Color32::from_rgb(darker(color.r()), darker(color.g()), darker(color.b()));
    }
    color
}

fn resolve(base: &Palette) -> Palette {
    let mut palette = *base;
    let [_, r, g, b] = SYSTEM_ACCENT.load(Ordering::Relaxed).to_be_bytes();
    if SYSTEM_ACCENT.load(Ordering::Relaxed) != 0 {
        palette.accent = Color32::from_rgb(r, g, b);
        palette.selection = readable_fill(palette.accent);
    }
    palette
}

pub fn palette(ui: &Ui) -> Palette {
    resolve(if ui.visuals().dark_mode { &DARK } else { &LIGHT })
}

/// Applies the system font and both appearances. The window follows the system setting.
pub fn install(ctx: &Context) {
    if let Ok(bytes) = std::fs::read(SYSTEM_FONT) {
        let mut fonts = FontDefinitions::default();
        fonts
            .font_data
            .insert("system".into(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "system".into());
        ctx.set_fonts(fonts);
    }
    if !sync_accent(ctx) {
        apply_styles(ctx);
    }
}

fn apply_styles(ctx: &Context) {
    ctx.style_mut_of(Theme::Dark, |style| apply(style, &resolve(&DARK)));
    ctx.style_mut_of(Theme::Light, |style| apply(style, &resolve(&LIGHT)));
}

/// Picks up the accent color from System Settings. Returns true if it changed.
pub fn sync_accent(ctx: &Context) -> bool {
    let Some(accent) = read_system_accent() else {
        return false;
    };
    let packed = u32::from_be_bytes([0xff, accent.r(), accent.g(), accent.b()]);
    if SYSTEM_ACCENT.swap(packed, Ordering::Relaxed) == packed {
        return false;
    }
    apply_styles(ctx);
    true
}

fn apply(style: &mut Style, p: &Palette) {
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(15.0)),
        (TextStyle::Body, FontId::proportional(BODY)),
        (TextStyle::Button, FontId::proportional(BODY)),
        (TextStyle::Small, FontId::proportional(SMALL)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(8.0, 6.0);
    spacing.button_padding = vec2(9.0, 3.0);
    // Controls are at least 22 pt tall; the macOS minimum is 20.
    spacing.interact_size = vec2(40.0, 22.0);
    spacing.slider_width = 150.0;
    spacing.slider_rail_height = 4.0;

    let v = &mut style.visuals;
    v.panel_fill = p.window;
    v.window_fill = p.popover;
    v.extreme_bg_color = p.plot;
    v.weak_text_color = Some(p.secondary);
    v.error_fg_color = p.danger;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, p.hairline);
    v.slider_trailing_fill = false;
    v.handle_shape = HandleShape::Circle;

    let radius = CornerRadius::same(6);
    let widgets = &mut v.widgets;
    widgets.noninteractive.bg_fill = p.window;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.hairline);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.label);
    widgets.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut widgets.inactive, p.control),
        (&mut widgets.hovered, p.control_hover),
        (&mut widgets.active, p.control_active),
        (&mut widgets.open, p.control_hover),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, p.label);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
}
