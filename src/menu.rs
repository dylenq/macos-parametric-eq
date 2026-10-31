use crate::state::{Mode, builtins};
use crate::tray::Tray;
use crate::ui::presets::PresetAction;
use eframe::egui;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Undo,
    Redo,
    ToggleBypass,
    SetMode(Mode),
    AddBand,
    DeleteBand,
    RemoveAllBands,
    Reset,
    TogglePerDevice,
    Preset(PresetAction),
    ShowWindow,
    ToggleLogin,
    Quit,
}

/// Menu item id of the preset at an index: "preset:3".
const PRESET_ID: &str = "preset:";

impl Command {
    /// Every command with a fixed menu item. Presets get theirs as they are created.
    const FIXED: [Command; 18] = [
        Command::Undo,
        Command::Redo,
        Command::ToggleBypass,
        Command::SetMode(Mode::Parametric),
        Command::SetMode(Mode::Graphic),
        Command::AddBand,
        Command::DeleteBand,
        Command::RemoveAllBands,
        Command::Reset,
        Command::TogglePerDevice,
        Command::Preset(PresetAction::Save),
        Command::Preset(PresetAction::SaveAs),
        Command::Preset(PresetAction::Rename),
        Command::Preset(PresetAction::Delete),
        Command::Preset(PresetAction::Import),
        Command::ShowWindow,
        Command::ToggleLogin,
        Command::Quit,
    ];

    pub fn id(self) -> String {
        match self {
            Command::Undo => "undo".into(),
            Command::Redo => "redo".into(),
            Command::ToggleBypass => "bypass".into(),
            Command::SetMode(Mode::Parametric) => "parametric".into(),
            Command::SetMode(Mode::Graphic) => "graphic".into(),
            Command::AddBand => "add-band".into(),
            Command::DeleteBand => "delete-band".into(),
            Command::RemoveAllBands => "remove-all-bands".into(),
            Command::Reset => "reset".into(),
            Command::TogglePerDevice => "per-device".into(),
            Command::Preset(PresetAction::Select(index)) => format!("{PRESET_ID}{index}"),
            Command::Preset(PresetAction::Save) => "save-preset".into(),
            Command::Preset(PresetAction::SaveAs) => "save-preset-as".into(),
            Command::Preset(PresetAction::Rename) => "rename-preset".into(),
            Command::Preset(PresetAction::Delete) => "delete-preset".into(),
            // Only the preset picker's rows rename and delete a preset other than the active
            // one, so these never become menu items.
            Command::Preset(PresetAction::RenameAt(index)) => format!("rename-preset:{index}"),
            Command::Preset(PresetAction::DeleteAt(index)) => format!("delete-preset:{index}"),
            Command::Preset(PresetAction::Import) => "import-preset".into(),
            Command::ShowWindow => "show".into(),
            Command::ToggleLogin => "login".into(),
            Command::Quit => "quit".into(),
        }
    }

    fn from_id(id: &str) -> Option<Command> {
        match id.strip_prefix(PRESET_ID) {
            Some(index) => Some(Command::Preset(PresetAction::Select(index.parse().ok()?))),
            None => Command::FIXED.into_iter().find(|c| c.id() == id),
        }
    }
}

/// What the menu needs to know to enable and check its items.
#[derive(Clone, PartialEq, Eq)]
pub struct MenuState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub bypass: bool,
    pub mode: Mode,
    pub can_add: bool,
    pub has_selection: bool,
    pub can_remove_all: bool,
    pub can_reset: bool,
    pub per_device: bool,
    /// Every preset, built-in ones first.
    pub presets: Vec<String>,
    pub active_preset: Option<usize>,
    pub can_save_preset: bool,
    /// The active preset is one of the user's own, which can be renamed and deleted.
    pub can_change_preset: bool,
    pub open_at_login: bool,
}

/// One checkable menu item per preset, with a separator after the built-in presets and
/// another after the user's own.
#[derive(Default)]
pub struct PresetItems {
    items: Vec<CheckMenuItem>,
    separators: Vec<PredefinedMenuItem>,
}

impl PresetItems {
    /// Replaces the items, which start at `position` in `menu`.
    pub fn list(&mut self, menu: &Submenu, position: usize, names: &[String]) {
        // Menu edits only fail for items that are not in the menu, which these always are.
        for item in self.items.drain(..) {
            _ = menu.remove(&item);
        }
        for separator in self.separators.drain(..) {
            _ = menu.remove(&separator);
        }
        let mut at = position;
        for (index, name) in names.iter().enumerate() {
            if index == builtins().len() {
                let separator = PredefinedMenuItem::separator();
                _ = menu.insert(&separator, at);
                self.separators.push(separator);
                at += 1;
            }
            let id = Command::Preset(PresetAction::Select(index)).id();
            let item = CheckMenuItem::with_id(id, name, true, false, None);
            _ = menu.insert(&item, at);
            self.items.push(item);
            at += 1;
        }
        if !names.is_empty() {
            let separator = PredefinedMenuItem::separator();
            _ = menu.insert(&separator, at);
            self.separators.push(separator);
        }
    }

    pub fn check(&self, active: Option<usize>) {
        for (index, item) in self.items.iter().enumerate() {
            item.set_checked(active == Some(index));
        }
    }
}

/// The macOS menu bar, so every command in the window is also reachable from a menu, and
/// the menu bar item that stays when the window is closed.
pub struct AppMenu {
    _menu: Menu,
    undo: MenuItem,
    redo: MenuItem,
    bypass: CheckMenuItem,
    parametric: CheckMenuItem,
    graphic: CheckMenuItem,
    add: MenuItem,
    delete: MenuItem,
    remove_all: MenuItem,
    reset: MenuItem,
    per_device: CheckMenuItem,
    presets: Submenu,
    preset_items: PresetItems,
    open_at_login: CheckMenuItem,
    /// Absent only if the menu bar item could not be created.
    tray: Option<Tray>,
    save_preset: MenuItem,
    rename_preset: MenuItem,
    delete_preset: MenuItem,
    commands: mpsc::Receiver<Command>,
    shown: Option<MenuState>,
    /// A chosen check item flips its own mark, which may no longer match `shown`.
    stale: bool,
}

fn command_key(code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Modifiers::META, code))
}

impl AppMenu {
    /// Builds the menu bar. Must run on the main thread after the app has launched.
    pub fn install(ctx: &egui::Context) -> Result<AppMenu, muda::Error> {
        let item = |command: Command, text: &str, key: Option<Accelerator>| {
            MenuItem::with_id(command.id(), text, true, key)
        };
        let undo = item(Command::Undo, "Undo", command_key(Code::KeyZ));
        let redo = item(
            Command::Redo,
            "Redo",
            Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyZ)),
        );
        let bypass = CheckMenuItem::with_id(
            Command::ToggleBypass.id(),
            "Bypass",
            true,
            false,
            command_key(Code::KeyB),
        );
        let mode = |mode: Mode, text: &str, key: Code| {
            CheckMenuItem::with_id(Command::SetMode(mode).id(), text, true, false, command_key(key))
        };
        let parametric = mode(Mode::Parametric, "Parametric", Code::Digit1);
        let graphic = mode(Mode::Graphic, "Graphic", Code::Digit2);
        let add = item(Command::AddBand, "Add Band", command_key(Code::KeyN));
        let delete = item(Command::DeleteBand, "Delete Band", None);
        let remove_all = item(Command::RemoveAllBands, "Remove All Bands", None);
        let reset = item(Command::Reset, "Reset", None);
        let per_device = CheckMenuItem::with_id(
            Command::TogglePerDevice.id(),
            "Separate EQ per Output Device",
            true,
            true,
            None,
        );
        let save_preset = item(
            Command::Preset(PresetAction::Save),
            "Save Preset",
            command_key(Code::KeyS),
        );
        let save_preset_as = item(
            Command::Preset(PresetAction::SaveAs),
            "Save Preset As…",
            Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyS)),
        );
        let rename_preset = item(Command::Preset(PresetAction::Rename), "Rename Preset…", None);
        let delete_preset = item(Command::Preset(PresetAction::Delete), "Delete Preset…", None);
        let import_preset = item(Command::Preset(PresetAction::Import), "Import…", None);
        let open_at_login =
            CheckMenuItem::with_id(Command::ToggleLogin.id(), "Open at Login", true, false, None);
        // A custom Quit so settings are saved and audio is restored before exiting.
        let quit = item(Command::Quit, "Quit EQ", command_key(Code::KeyQ));

        let separator = PredefinedMenuItem::separator;
        let app = Submenu::with_items(
            "EQ",
            true,
            &[
                &PredefinedMenuItem::about(Some("About EQ"), None),
                &separator(),
                &open_at_login,
                &separator(),
                &PredefinedMenuItem::hide(Some("Hide EQ")),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &separator(),
                &quit,
            ],
        )?;
        let edit = Submenu::with_items("Edit", true, &[&undo, &redo])?;
        let equalizer = Submenu::with_items(
            "Equalizer",
            true,
            &[
                &bypass,
                &separator(),
                &parametric,
                &graphic,
                &separator(),
                &add,
                &delete,
                &remove_all,
                &separator(),
                &reset,
                &separator(),
                &per_device,
            ],
        )?;
        let presets = Submenu::with_items(
            "Presets",
            true,
            &[
                &save_preset,
                &save_preset_as,
                &rename_preset,
                &delete_preset,
                &separator(),
                &import_preset,
            ],
        )?;
        let window = Submenu::with_items(
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(Some("Zoom")),
                &PredefinedMenuItem::fullscreen(None),
                &separator(),
                &PredefinedMenuItem::close_window(None),
            ],
        )?;
        let menu = Menu::with_items(&[&app, &edit, &equalizer, &presets, &window])?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();

        let (sender, commands) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = Command::from_id(&event.id().0) {
                _ = sender.send(command);
                ctx.request_repaint();
            }
        }));

        Ok(AppMenu {
            _menu: menu,
            undo,
            redo,
            bypass,
            parametric,
            graphic,
            add,
            delete,
            remove_all,
            reset,
            per_device,
            presets,
            preset_items: PresetItems::default(),
            open_at_login,
            tray: Tray::install()
                .inspect_err(|error| eprintln!("could not add the menu bar item: {error}"))
                .ok(),
            save_preset,
            rename_preset,
            delete_preset,
            commands,
            shown: None,
            stale: false,
        })
    }

    /// Commands chosen from the menu since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        let commands: Vec<Command> = self.commands.try_iter().collect();
        self.stale |= !commands.is_empty();
        commands
    }

    pub fn sync(&mut self, state: MenuState) {
        if self.shown.as_ref() == Some(&state) && !self.stale {
            return;
        }
        self.stale = false;
        let presets_changed = self.shown.as_ref().map(|s| &s.presets) != Some(&state.presets);
        if presets_changed {
            self.preset_items.list(&self.presets, 0, &state.presets);
        }
        if let Some(tray) = &mut self.tray {
            tray.sync(&state, presets_changed);
        }
        self.undo.set_enabled(state.can_undo);
        self.redo.set_enabled(state.can_redo);
        self.bypass.set_checked(state.bypass);
        self.parametric.set_checked(state.mode == Mode::Parametric);
        self.graphic.set_checked(state.mode == Mode::Graphic);
        self.add.set_enabled(state.can_add);
        self.delete.set_enabled(state.has_selection);
        self.remove_all.set_enabled(state.can_remove_all);
        self.reset.set_enabled(state.can_reset);
        self.per_device.set_checked(state.per_device);
        self.preset_items.check(state.active_preset);
        self.open_at_login.set_checked(state.open_at_login);
        self.save_preset.set_enabled(state.can_save_preset);
        self.rename_preset.set_enabled(state.can_change_preset);
        self.delete_preset.set_enabled(state.can_change_preset);
        self.shown = Some(state);
    }
}
