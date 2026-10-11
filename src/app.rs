use crate::appkit;
use crate::audio::devices;
use crate::audio::engine::Engine;
use crate::dsp::chain::ChainParams;
use crate::history::History;
use crate::import;
use crate::login;
use crate::menu::{AppMenu, Command, MenuState};
use crate::state::{MAX_BANDS, Settings};
use crate::ui::controls::{self, Level};
use crate::ui::presets::{Dialog, PresetAction};
use crate::ui::{curve, theme};
use eframe::egui::{self, CentralPanel, Frame, Margin, Panel, RichText, ViewportCommand};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const DEVICE_POLL: Duration = Duration::from_millis(500);
const ACCENT_POLL: Duration = Duration::from_secs(1);
const SAVE_DELAY: Duration = Duration::from_secs(1);
const CLIP_HOLD: Duration = Duration::from_secs(2);
const SILENCE_HINT_AFTER: Duration = Duration::from_secs(8);
const NOTICE_HOLD: Duration = Duration::from_secs(6);
const DEFAULT_SAMPLE_RATE: f64 = 48_000.0;
/// Meter fall rate in dB per second.
const METER_DECAY: f32 = 40.0;

struct Running {
    engine: Engine,
    /// The output device's own sample rate when the engine started.
    device_rate: Option<f64>,
    device_name: String,
}

pub struct App {
    settings: Settings,
    selected: Option<usize>,
    /// The preset sheet, while one is open.
    dialog: Option<Dialog>,
    running: Option<Running>,
    /// An engine start in progress on another thread.
    starting: Option<mpsc::Receiver<Result<Running, String>>>,
    error: Option<String>,
    /// The outcome of an import, shown in place of the device for a few seconds.
    notice: Option<(String, Instant)>,
    /// Parameters the audio thread has not accepted yet.
    unsent: bool,
    unsaved_since: Option<Instant>,
    last_device_poll: Option<Instant>,
    output_db: f32,
    clipped_at: Option<Instant>,
    /// When the engine first started, cleared once any audio arrives through the tap.
    waiting_for_audio: Option<Instant>,
    heard_audio: bool,
    last_frame: Instant,
    last_accent_poll: Instant,
    history: History,
    /// Absent only if the menu bar could not be built.
    menu: Option<AppMenu>,
    open_at_login: bool,
    /// The window is closed and EQ is running from the menu bar.
    hidden: bool,
    /// Another app has been in front since the window was hidden, so EQ coming to the
    /// foreground again means the user asked for it, from the Dock for instance.
    left_foreground: bool,
    quitting: bool,
}

impl App {
    pub fn new(ctx: &egui::Context, settings: Settings) -> Self {
        App {
            hidden: settings.window_hidden,
            // Opening the app by hand puts it in front, which shows the window at once.
            left_foreground: true,
            quitting: false,
            open_at_login: login::enabled(),
            settings,
            selected: None,
            dialog: None,
            running: None,
            starting: None,
            error: None,
            notice: None,
            unsent: false,
            unsaved_since: None,
            last_device_poll: None,
            output_db: -120.0,
            clipped_at: None,
            waiting_for_audio: None,
            heard_audio: false,
            last_frame: Instant::now(),
            last_accent_poll: Instant::now(),
            history: History::default(),
            menu: AppMenu::install(ctx)
                .inspect_err(|error| eprintln!("could not build the menu bar: {error}"))
                .ok(),
        }
    }

    fn sample_rate(&self) -> f64 {
        self.running
            .as_ref()
            .map_or(DEFAULT_SAMPLE_RATE, |r| r.engine.sample_rate)
    }

    /// Starts the engine, and restarts it when the default output device or its sample
    /// rate changes.
    fn follow_output_device(&mut self) {
        self.finish_starting();
        if self.starting.is_some()
            || self.last_device_poll.is_some_and(|t| t.elapsed() < DEVICE_POLL)
        {
            return;
        }
        self.last_device_poll = Some(Instant::now());

        let Some(device) = devices::default_output_device() else {
            self.running = None;
            self.error = Some("No output device".into());
            return;
        };
        let device_rate = devices::sample_rate(device);
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.engine.output_device == device && r.device_rate == device_rate)
        {
            return;
        }

        if let Some(uid) = devices::device_uid(device)
            && self.settings.switch_device(&uid.to_string())
        {
            // Each device has its own curve, so undo does not reach across the switch.
            self.history = History::default();
            self.selected = None;
            self.mark_changed();
        }

        // The old tap must be gone before a new one mutes the system again.
        self.running = None;
        // Starting can block for as long as the system's audio capture prompt is open, so
        // it runs off the UI thread and the window stays responsive.
        let (sender, receiver) = mpsc::channel();
        let state = self.settings.eq.clone();
        std::thread::spawn(move || {
            let started = Engine::start(device, |rate| ChainParams::from_state(&state, rate))
                .map(|engine| Running {
                    engine,
                    device_rate,
                    device_name: devices::device_name(device).unwrap_or_else(|| "Output".into()),
                });
            _ = sender.send(started);
        });
        self.starting = Some(receiver);
    }

    fn finish_starting(&mut self) {
        let Some(result) = self.starting.as_ref().and_then(|r| r.try_recv().ok()) else {
            return;
        };
        self.starting = None;
        match result {
            Ok(running) => {
                self.running = Some(running);
                self.error = None;
                // The bands may have changed while the engine was starting.
                self.unsent = true;
                if !self.heard_audio {
                    self.waiting_for_audio.get_or_insert_with(Instant::now);
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn read_meters(&mut self, dt: f32) {
        self.output_db = (self.output_db - METER_DECAY * dt).max(-120.0);
        let Some(running) = &self.running else {
            return;
        };
        let (input, output) = running.engine.meters.take();
        if input > 1e-5 {
            self.waiting_for_audio = None;
            self.heard_audio = true;
        }
        if output >= 1.0 {
            self.clipped_at = Some(Instant::now());
        }
        self.output_db = self.output_db.max(20.0 * output.max(1e-6).log10());
    }

    fn status(&self) -> String {
        if let Some((notice, shown)) = &self.notice
            && shown.elapsed() < NOTICE_HOLD
        {
            return notice.clone();
        }
        match (&self.running, &self.error) {
            (Some(r), _) => format!("{} · {:.1} kHz", r.device_name, r.engine.sample_rate / 1000.0),
            (None, Some(error)) => error.clone(),
            (None, None) => "Waiting for audio access…".into(),
        }
    }

    fn push_changes(&mut self) {
        if self.unsent
            && let Some(running) = &mut self.running
        {
            let params = ChainParams::from_state(&self.settings.eq, running.engine.sample_rate);
            self.unsent = !running.engine.send(params);
        }
        if self.unsaved_since.is_some_and(|t| t.elapsed() >= SAVE_DELAY) {
            self.save();
        }
    }

    fn mark_changed(&mut self) {
        self.unsent = true;
        self.unsaved_since = Some(Instant::now());
    }

    fn preset_action(&mut self, action: PresetAction) {
        match action {
            PresetAction::Select(index) => {
                self.settings.apply_preset(index);
                self.selected = None;
            }
            PresetAction::Save => match self.settings.active_preset.clone() {
                Some(name) => self.settings.save_preset(&name),
                None => self.dialog = Some(Dialog::save_as(&self.settings)),
            },
            PresetAction::SaveAs => self.dialog = Some(Dialog::save_as(&self.settings)),
            PresetAction::Rename => self.dialog = Dialog::rename(&self.settings),
            PresetAction::Delete => self.dialog = Dialog::delete(&self.settings),
            PresetAction::Import => {
                let file = rfd::FileDialog::new()
                    .set_title("Import an AutoEq or Equalizer APO file")
                    .add_filter("Equalizer APO", &["txt"])
                    .pick_file();
                if let Some(path) = file {
                    self.import(&path);
                }
            }
        }
    }

    /// Loads an Equalizer APO file as a preset named after it.
    fn import(&mut self, path: &Path) {
        let notice = match import::read(path) {
            Ok((name, imported)) => {
                self.settings.import_preset(&name, &imported.curve);
                self.selected = None;
                match imported.skipped {
                    0 => format!("Imported “{name}”"),
                    1 => format!("Imported “{name}”, 1 filter skipped"),
                    n => format!("Imported “{name}”, {n} filters skipped"),
                }
            }
            Err(error) => error,
        };
        self.notice = Some((notice, Instant::now()));
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        self.settings.window_hidden = false;
        self.mark_changed();
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        appkit::activate();
    }

    /// Closes the window but keeps equalizing, reachable from the menu bar and the Dock.
    fn hide_window(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        self.left_foreground = false;
        self.settings.window_hidden = true;
        self.mark_changed();
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        appkit::hide();
    }

    fn toggle_login(&mut self) {
        if let Err(error) = login::set_enabled(!self.open_at_login) {
            self.notice = Some((format!("Could not change Open at Login: {error}"), Instant::now()));
        }
        self.open_at_login = login::enabled();
    }

    /// Runs what was chosen from the menus since the last call. A drag in the window can
    /// span this call, and `gesture_active` keeps it from ending the drag's undo step.
    fn run_commands(&mut self, ctx: &egui::Context, gesture_active: bool) {
        let commands = self.menu.as_mut().map(AppMenu::take_commands).unwrap_or_default();
        // Undo and redo replace the state outright, so they are not themselves recorded.
        for command in &commands {
            let changed = match command {
                Command::Undo => self.history.undo(&mut self.settings.eq),
                Command::Redo => self.history.redo(&mut self.settings.eq),
                _ => false,
            };
            if changed {
                self.selected = None;
                self.mark_changed();
            }
        }

        let before = self.settings.clone();
        for command in commands {
            match command {
                Command::ToggleBypass => self.settings.eq.bypass = !self.settings.eq.bypass,
                Command::AddBand => self.settings.eq.add_band(&mut self.selected),
                Command::DeleteBand => {
                    if let Some(index) = self.selected.take()
                        && index < self.settings.eq.bands.len()
                    {
                        self.settings.eq.bands.remove(index);
                    }
                }
                Command::Reset => {
                    self.settings.eq.reset_bands();
                    self.selected = None;
                }
                Command::TogglePerDevice => self.settings.per_device = !self.settings.per_device,
                Command::Preset(action) => {
                    // The sheets and the file panel need the window.
                    if self.hidden && !matches!(action, PresetAction::Select(_)) {
                        self.show_window(ctx);
                    }
                    self.preset_action(action);
                }
                Command::ShowWindow => self.show_window(ctx),
                Command::ToggleLogin => self.toggle_login(),
                Command::Quit => {
                    self.quitting = true;
                    self.save();
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Command::Undo | Command::Redo => {}
            }
        }
        self.finish_edits(&before, gesture_active);
    }

    /// Call with the settings from before a round of edits, to save, send and record them.
    fn finish_edits(&mut self, before: &Settings, gesture_active: bool) {
        if self.settings != *before {
            self.settings.eq.bands.iter_mut().for_each(|b| b.clamp());
            self.mark_changed();
        }
        self.history.observe(&before.eq, &self.settings.eq, gesture_active);
        self.push_changes();
    }

    fn sync_menu(&mut self) {
        let Some(menu) = &mut self.menu else {
            return;
        };
        // While a sheet is open, ⌘Z belongs to its text field.
        let no_sheet = self.dialog.is_none();
        menu.sync(MenuState {
            can_undo: no_sheet && self.history.can_undo(),
            can_redo: no_sheet && self.history.can_redo(),
            bypass: self.settings.eq.bypass,
            can_add: self.settings.eq.bands.len() < MAX_BANDS,
            has_selection: self.selected.is_some(),
            per_device: self.settings.per_device,
            presets: self.settings.presets.iter().map(|p| p.name.clone()).collect(),
            active_preset: self.settings.active_index(),
            preset_modified: self.settings.is_modified(),
            open_at_login: self.open_at_login,
        });
    }

    fn save(&mut self) {
        if self.unsaved_since.take().is_some()
            && let Err(error) = self.settings.save()
        {
            eprintln!("could not save settings: {error}");
        }
    }
}

impl eframe::App for App {
    /// Everything that has to keep going while the window is closed.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dt = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        self.follow_output_device();
        self.read_meters(dt);

        if self.hidden {
            if !appkit::is_active() {
                self.left_foreground = true;
            } else if self.left_foreground {
                self.show_window(ctx);
            }
        }
        let pointer_down = !self.hidden && ctx.input(|i| i.pointer.any_down());
        self.run_commands(ctx, pointer_down);
        self.sync_menu();
        // Keeps the meter moving and the device poll running while idle.
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.last_accent_poll.elapsed() >= ACCENT_POLL {
            self.last_accent_poll = Instant::now();
            theme::sync_accent(ui.ctx());
        }
        // Closing the window leaves EQ running in the menu bar; only Quit stops it.
        if ui.input(|i| i.viewport().close_requested()) && !self.quitting {
            ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide_window(ui.ctx());
        }

        let before = self.settings.clone();
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        for file in &dropped {
            self.import(file.path());
        }

        let level = Level {
            output_db: self.output_db,
            clipped: self.clipped_at.is_some_and(|t| t.elapsed() < CLIP_HOLD),
        };
        let status = self.status();
        let silent = self.running.is_some()
            && self.waiting_for_audio.is_some_and(|t| t.elapsed() > SILENCE_HINT_AFTER);
        let sample_rate = self.sample_rate();

        let mut picked = None;
        let p = &theme::palette(ui);
        let panel = |margin: Margin| Frame::new().fill(p.window).inner_margin(margin);
        // The top margin clears the transparent title bar and its window buttons.
        Panel::top("top")
            .frame(panel(Margin { left: 16, right: 16, top: 38, bottom: 10 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                picked = controls::top_bar(ui, &mut self.settings, &level, &status);
                if silent {
                    ui.label(
                        RichText::new(
                            "No audio detected. If something is playing, allow EQ in System \
                             Settings > Privacy & Security > Screen & System Audio Recording.",
                        )
                        .small()
                        .color(p.secondary),
                    );
                }
            });
        Panel::bottom("bands")
            .frame(panel(Margin { left: 8, right: 16, top: 8, bottom: 12 }))
            .show_separator_line(false)
            .resizable(true)
            .default_size(244.0)
            .min_size(96.0)
            .show(ui, |ui| {
                controls::band_list(ui, &mut self.settings.eq, &mut self.selected);
            });
        CentralPanel::default()
            .frame(panel(Margin { left: 16, right: 16, top: 4, bottom: 4 }))
            .show(ui, |ui| {
                curve::show(ui, &mut self.settings.eq, &mut self.selected, sample_rate);
            });

        if let Some(action) = picked {
            self.preset_action(action);
        }
        if let Some(dialog) = &mut self.dialog
            && !dialog.show(ui.ctx(), &mut self.settings)
        {
            self.dialog = None;
        }

        let pointer_down = ui.input(|i| i.pointer.any_down());
        self.finish_edits(&before, pointer_down);
        self.sync_menu();
    }

    fn on_exit(&mut self) {
        self.save();
        // Stop processing before the window goes away so system audio is restored cleanly.
        self.running = None;
    }
}
