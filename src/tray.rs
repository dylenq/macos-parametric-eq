use crate::menu::{Command, MenuState, PresetItems};
use muda::{CheckMenuItem, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Pixels per side: 18 pt, the height of a menu bar icon, at 2x.
const ICON_SIZE: u32 = 36;
/// Where the preset items go in the menu: after Bypass and its separator.
const PRESETS_AT: usize = 2;

/// The menu bar item, which keeps EQ within reach while its window is closed.
pub struct Tray {
    _icon: TrayIcon,
    menu: Submenu,
    bypass: CheckMenuItem,
    presets: PresetItems,
    login: CheckMenuItem,
}

/// Three sliders at different heights, drawn as a template image so the system colors it.
fn icon() -> Icon {
    const SLIDERS: [(f32, f32); 3] = [(8.0, 23.0), (18.0, 11.0), (28.0, 19.0)];
    let mut rgba = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let distance = SLIDERS
                .iter()
                .map(|&(sx, knob)| {
                    let track = (px - sx).hypot(py - py.clamp(5.0, 31.0)) - 1.25;
                    let knob = (px - sx).hypot(py - knob) - 4.5;
                    track.min(knob)
                })
                .fold(f32::MAX, f32::min);
            let alpha = (0.5 - distance).clamp(0.0, 1.0);
            rgba.extend([0, 0, 0, (alpha * 255.0) as u8]);
        }
    }
    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).expect("the buffer matches the icon size")
}

impl Tray {
    /// Adds the item to the menu bar. Must run on the main thread after the app has launched.
    pub fn install() -> Result<Tray, Box<dyn std::error::Error>> {
        let bypass =
            CheckMenuItem::with_id(Command::ToggleBypass.id(), "Bypass", true, false, None);
        let show = MenuItem::with_id(Command::ShowWindow.id(), "Show EQ", true, None);
        let login =
            CheckMenuItem::with_id(Command::ToggleLogin.id(), "Open at Login", true, false, None);
        let quit = MenuItem::with_id(Command::Quit.id(), "Quit EQ", true, None);
        let separator = PredefinedMenuItem::separator;
        let menu = Submenu::with_items(
            "EQ",
            true,
            &[&bypass, &separator(), &show, &login, &separator(), &quit],
        )?;
        let icon = TrayIconBuilder::new()
            .with_icon_templated(icon())
            .with_tooltip("EQ")
            .with_menu(Box::new(menu.clone()))
            .build()?;
        Ok(Tray {
            _icon: icon,
            menu,
            bypass,
            presets: PresetItems::default(),
            login,
        })
    }

    pub fn sync(&mut self, state: &MenuState, presets_changed: bool) {
        if presets_changed {
            self.presets.list(&self.menu, PRESETS_AT, &state.presets);
        }
        self.presets.check(state.active_preset);
        self.bypass.set_checked(state.bypass);
        self.login.set_checked(state.open_at_login);
    }
}
