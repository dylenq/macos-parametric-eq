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
    AddBand,
    DeleteBand,
    Reset,
    TogglePerDevice,
    Preset(PresetAction),
    Quit,
}

/// Menu item id of the preset at an index: "preset:3".
const PRESET_ID: &str = "preset:";

impl Command {
    /// Every command with a fixed menu item. Presets get theirs as they are created.
    const FIXED: [Command; 12] = [
        Command::Undo,
        Command::Redo,
        Command::ToggleBypass,
        Command::AddBand,
        Command::DeleteBand,
        Command::Reset,
        Command::TogglePerDevice,
        Command::Preset(PresetAction::Save),
        Command::Preset(PresetAction::SaveAs),
        Command::Preset(PresetAction::Rename),
        Command::Preset(PresetAction::Delete),
        Command::Quit,
    ];

    fn id(self) -> String {
        match self {
            Command::Undo => "undo".into(),
            Command::Redo => "redo".into(),
            Command::ToggleBypass => "bypass".into(),
            Command::AddBand => "add-band".into(),
            Command::DeleteBand => "delete-band".into(),
            Command::Reset => "reset".into(),
            Command::TogglePerDevice => "per-device".into(),
            Command::Preset(PresetAction::Select(index)) => format!("{PRESET_ID}{index}"),
            Command::Preset(PresetAction::Save) => "save-preset".into(),
            Command::Preset(PresetAction::SaveAs) => "save-preset-as".into(),
            Command::Preset(PresetAction::Rename) => "rename-preset".into(),
            Command::Preset(PresetAction::Delete) => "delete-preset".into(),
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
    pub can_add: bool,
    pub has_selection: bool,
    pub per_device: bool,
    pub presets: Vec<String>,
    pub active_preset: Option<usize>,
    pub preset_modified: bool,
}

/// The macOS menu bar, so every command in the window is also reachable from a menu.
pub struct AppMenu {
    _menu: Menu,
    undo: MenuItem,
    redo: MenuItem,
    bypass: CheckMenuItem,
    add: MenuItem,
    delete: MenuItem,
    per_device: CheckMenuItem,
    presets: Submenu,
    /// One item per preset at the top of the Presets menu, then a separator if there are any.
    preset_items: Vec<CheckMenuItem>,
    preset_separator: PredefinedMenuItem,
    save_preset: MenuItem,
    rename_preset: MenuItem,
    delete_preset: MenuItem,
    commands: mpsc::Receiver<Command>,
    shown: Option<MenuState>,
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
        let add = item(Command::AddBand, "Add Band", command_key(Code::KeyN));
        let delete = item(Command::DeleteBand, "Delete Band", None);
        let reset = item(Command::Reset, "Reset Bands", None);
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
        // A custom Quit so settings are saved and audio is restored before exiting.
        let quit = item(Command::Quit, "Quit EQ", command_key(Code::KeyQ));

        let separator = PredefinedMenuItem::separator;
        let app = Submenu::with_items(
            "EQ",
            true,
            &[
                &PredefinedMenuItem::about(Some("About EQ"), None),
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
                &add,
                &delete,
                &separator(),
                &reset,
                &separator(),
                &per_device,
            ],
        )?;
        let presets = Submenu::with_items(
            "Presets",
            true,
            &[&save_preset, &save_preset_as, &rename_preset, &delete_preset],
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
            add,
            delete,
            per_device,
            presets,
            preset_items: Vec::new(),
            preset_separator: separator(),
            save_preset,
            rename_preset,
            delete_preset,
            commands,
            shown: None,
        })
    }

    /// Commands chosen from the menu since the last call.
    pub fn take_commands(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }

    pub fn sync(&mut self, state: MenuState) {
        if self.shown.as_ref() == Some(&state) {
            return;
        }
        if self.shown.as_ref().map(|s| &s.presets) != Some(&state.presets) {
            self.list_presets(&state.presets);
        }
        self.undo.set_enabled(state.can_undo);
        self.redo.set_enabled(state.can_redo);
        self.bypass.set_checked(state.bypass);
        self.add.set_enabled(state.can_add);
        self.delete.set_enabled(state.has_selection);
        self.per_device.set_checked(state.per_device);
        for (index, item) in self.preset_items.iter().enumerate() {
            item.set_checked(state.active_preset == Some(index));
        }
        self.save_preset.set_enabled(state.preset_modified);
        self.rename_preset.set_enabled(state.active_preset.is_some());
        self.delete_preset.set_enabled(state.active_preset.is_some());
        self.shown = Some(state);
    }

    /// Replaces the preset items at the top of the Presets menu.
    fn list_presets(&mut self, names: &[String]) {
        // Menu edits only fail for items that are not in the menu, which these always are.
        for item in self.preset_items.drain(..) {
            _ = self.presets.remove(&item);
        }
        _ = self.presets.remove(&self.preset_separator);
        for (index, name) in names.iter().enumerate() {
            let id = Command::Preset(PresetAction::Select(index)).id();
            let item = CheckMenuItem::with_id(id, name, true, false, None);
            _ = self.presets.insert(&item, index);
            self.preset_items.push(item);
        }
        if !names.is_empty() {
            _ = self.presets.insert(&self.preset_separator, names.len());
        }
    }
}
