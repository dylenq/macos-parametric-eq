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
    Quit,
}

impl Command {
    const ALL: [Command; 7] = [
        Command::Undo,
        Command::Redo,
        Command::ToggleBypass,
        Command::AddBand,
        Command::DeleteBand,
        Command::Reset,
        Command::Quit,
    ];

    fn id(self) -> &'static str {
        match self {
            Command::Undo => "undo",
            Command::Redo => "redo",
            Command::ToggleBypass => "bypass",
            Command::AddBand => "add-band",
            Command::DeleteBand => "delete-band",
            Command::Reset => "reset",
            Command::Quit => "quit",
        }
    }
}

/// What the menu needs to know to enable and check its items.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MenuState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub bypass: bool,
    pub can_add: bool,
    pub has_selection: bool,
}

/// The macOS menu bar, so every command in the window is also reachable from a menu.
pub struct AppMenu {
    _menu: Menu,
    undo: MenuItem,
    redo: MenuItem,
    bypass: CheckMenuItem,
    add: MenuItem,
    delete: MenuItem,
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
            &[&bypass, &separator(), &add, &delete, &separator(), &reset],
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
        let menu = Menu::with_items(&[&app, &edit, &equalizer, &window])?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();

        let (sender, commands) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = Command::ALL.into_iter().find(|c| event.id() == c.id()) {
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
            commands,
            shown: None,
        })
    }

    /// Commands chosen from the menu since the last call.
    pub fn take_commands(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }

    pub fn sync(&mut self, state: MenuState) {
        if self.shown == Some(state) {
            return;
        }
        self.shown = Some(state);
        self.undo.set_enabled(state.can_undo);
        self.redo.set_enabled(state.can_redo);
        self.bypass.set_checked(state.bypass);
        self.add.set_enabled(state.can_add);
        self.delete.set_enabled(state.has_selection);
    }
}
