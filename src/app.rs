use crate::appkit;
use crate::audio::devices;
use crate::audio::engine::Engine;
use crate::dsp::chain::ChainParams;
use crate::history::History;
use crate::import;
use crate::login;
use crate::menu::{AppMenu, Command, MenuState};
use crate::state::{LoadProblem, MAX_BANDS, Mode, Settings};
use crate::ui::controls::{Level, Status};
use crate::ui::presets::{self, Dialog, Outcome, PresetAction};
use crate::ui::{self, View, theme};
use eframe::egui::{self, ViewportCommand};
use objc2_core_audio::AudioObjectID;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const DEVICE_POLL: Duration = Duration::from_millis(500);
/// Frame interval while the meter moves. Otherwise the window wakes at the device poll.
const METER_FRAME: Duration = Duration::from_millis(50);
const ACCENT_POLL: Duration = Duration::from_secs(1);
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// After a failed save, when to try again.
const SAVE_RETRY: Duration = Duration::from_secs(10);
/// Pauses before retrying an output that would not start: the first, doubling to the last.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(60);
const CLIP_HOLD: Duration = Duration::from_secs(2);
const SILENCE_HINT_AFTER: Duration = Duration::from_secs(8);
const NOTICE_HOLD: Duration = Duration::from_secs(6);
/// How long a message about lost or unsaved settings stays.
const WARNING_HOLD: Duration = Duration::from_secs(20);
const DEFAULT_SAMPLE_RATE: f64 = 48_000.0;
/// Meter fall rate in dB per second.
const METER_DECAY: f32 = 40.0;
/// The meter's empty end. Below it there is nothing to animate.
const METER_FLOOR_DB: f32 = -60.0;

/// A message shown in place of the device for a while.
struct Notice {
    text: String,
    shown: Instant,
    hold: Duration,
    /// The history serial at which the message's Undo still undoes its edit.
    undo: Option<u64>,
}

struct Running {
    engine: Engine,
    /// The output device's own sample rate when the engine started.
    device_rate: Option<f64>,
    device_name: String,
}

/// An engine start in progress on another thread, and the output it is for. It returns
/// once something plays, or once the system's audio capture prompt is answered.
struct Starting {
    receiver: mpsc::Receiver<Result<Running, String>>,
    device: AudioObjectID,
    rate: Option<f64>,
    since: Instant,
}

/// The output that last failed, which is retried less and less often until it works or
/// another output takes over.
struct Failure {
    device: AudioObjectID,
    rate: Option<f64>,
    count: u32,
    retry_at: Instant,
}

pub struct App {
    settings: Settings,
    selected: Option<usize>,
    /// The preset sheet, while one is open.
    dialog: Option<Dialog>,
    running: Option<Running>,
    starting: Option<Starting>,
    failure: Option<Failure>,
    error: Option<String>,
    notice: Option<Notice>,
    /// Why the settings could not be loaded, shown once the window is first seen.
    startup_warning: Option<String>,
    /// A message for an edit made this frame, waiting for the edit to become an undo step.
    undoable: Option<String>,
    /// The Undo next to a message was clicked. It runs with the menu commands, before the
    /// next round of edits, so it is not itself recorded as an edit.
    undo_requested: bool,
    /// Parameters the audio thread has not accepted yet.
    unsent: bool,
    /// When to write the settings, which have changed since they were last saved.
    save_at: Option<Instant>,
    /// The last save failed and the user has been told.
    save_failed: bool,
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
    pub fn new(ctx: &egui::Context, settings: Settings, problem: Option<LoadProblem>) -> Self {
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
            failure: None,
            error: None,
            notice: None,
            startup_warning: problem.map(|p| p.to_string()),
            undoable: None,
            undo_requested: false,
            unsent: false,
            save_at: None,
            save_failed: false,
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
    /// rate changes, or when processing stopped.
    fn follow_output_device(&mut self) {
        self.finish_starting();
        if let Some(running) = &self.running
            && running.engine.meters.failed()
        {
            let (device, rate) = (running.engine.output_device, running.device_rate);
            self.running = None;
            self.fail(device, rate, "Audio processing stopped unexpectedly".into());
        }
        if self.last_device_poll.is_some_and(|t| t.elapsed() < DEVICE_POLL) {
            return;
        }
        self.last_device_poll = Some(Instant::now());

        let Some(device) = devices::default_output_device() else {
            self.running = None;
            self.starting = None;
            self.error = Some("No output device".into());
            return;
        };
        let device_rate = devices::sample_rate(device);
        // A start waiting for an output that is no longer the default is dropped. Its thread
        // then cannot hand the engine over, and drops it as soon as the start returns.
        if let Some(starting) = &self.starting {
            if starting.device == device && starting.rate == device_rate {
                return;
            }
            self.starting = None;
        }
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.engine.output_device == device && r.device_rate == device_rate)
        {
            return;
        }
        // An output that failed is tried again after a pause, not on every poll.
        if self
            .failure
            .as_ref()
            .is_some_and(|f| f.device == device && f.rate == device_rate && Instant::now() < f.retry_at)
        {
            return;
        }

        if let Some(uid) = devices::device_uid(device)
            && self.settings.switch_device(&uid.to_string())
        {
            // Each device has its own curve, so undo does not reach across the switch.
            self.history.clear();
            self.notice = None;
            self.selected = None;
            self.mark_changed();
        }

        // The old tap must be gone before a new one mutes the system again.
        self.running = None;
        // An error stays up while its output is retried, but not for another output.
        if !self.failure.as_ref().is_some_and(|f| f.device == device && f.rate == device_rate) {
            self.error = None;
        }
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
        self.starting = Some(Starting {
            receiver,
            device,
            rate: device_rate,
            since: Instant::now(),
        });
    }

    fn finish_starting(&mut self) {
        let Some(starting) = &self.starting else {
            return;
        };
        let result = match starting.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            // The thread ended without an answer, which only a panic does.
            Err(mpsc::TryRecvError::Disconnected) => Err("Could not start audio processing".into()),
        };
        let Some(Starting { device, rate, .. }) = self.starting.take() else {
            return;
        };
        match result {
            Ok(running) => {
                self.running = Some(running);
                self.error = None;
                self.failure = None;
                // The bands may have changed while the engine was starting.
                self.unsent = true;
                if !self.heard_audio {
                    self.waiting_for_audio.get_or_insert_with(Instant::now);
                }
            }
            Err(error) => self.fail(device, rate, error),
        }
    }

    /// Shows why the output could not be used, and sets when to try it again.
    fn fail(&mut self, device: AudioObjectID, rate: Option<f64>, error: String) {
        let count = self
            .failure
            .as_ref()
            .filter(|f| f.device == device && f.rate == rate)
            .map_or(1, |f| f.count + 1);
        let delay = RETRY_FIRST.saturating_mul(1 << (count - 1).min(6)).min(RETRY_MAX);
        self.failure = Some(Failure {
            device,
            rate,
            count,
            retry_at: Instant::now() + delay,
        });
        self.error = Some(error);
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

    fn status(&self) -> Status {
        if let Some(notice) = &self.notice
            && notice.shown.elapsed() < notice.hold
        {
            // Undo is offered only while nothing else has been done since.
            let undo = notice.undo.is_some_and(|serial| {
                serial == self.history.serial() && !self.history.has_pending()
            });
            return Status {
                text: notice.text.clone(),
                undo,
            };
        }
        let text = match (&self.running, &self.error) {
            (Some(r), _) => format!("{} · {:.1} kHz", r.device_name, r.engine.sample_rate / 1000.0),
            (None, Some(error)) => error.clone(),
            // Waiting for something to play, or for the audio capture prompt to be answered.
            (None, None) => "Waiting for audio…".into(),
        };
        Status { text, undo: false }
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.notify_for(text, NOTICE_HOLD);
    }

    fn notify_for(&mut self, text: impl Into<String>, hold: Duration) {
        self.notice = Some(Notice {
            text: text.into(),
            shown: Instant::now(),
            hold,
            undo: None,
        });
    }

    fn push_changes(&mut self) {
        if self.unsent
            && let Some(running) = &mut self.running
        {
            let params = ChainParams::from_state(&self.settings.eq, running.engine.sample_rate);
            self.unsent = !running.engine.send(params);
        }
        if self.save_at.is_some_and(|t| Instant::now() >= t) {
            self.save();
        }
    }

    fn mark_changed(&mut self) {
        self.unsent = true;
        self.save_at = Some(Instant::now() + SAVE_DELAY);
    }

    fn preset_action(&mut self, action: PresetAction) {
        let own = presets::active_user_preset(&self.settings);
        match action {
            PresetAction::Select(index) => {
                let Some(name) = self.settings.preset(index).map(|p| p.name.clone()) else {
                    return;
                };
                // Applying a preset is an undo step of its own.
                self.history.commit();
                let before = self.settings.eq.curve();
                self.settings.apply_preset(index);
                self.selected = None;
                let text = format!("Applied {name}");
                if self.settings.eq.curve() == before {
                    self.notify(text);
                } else {
                    self.undoable = Some(text);
                }
            }
            PresetAction::Save => match own.and_then(|i| self.settings.preset(i)) {
                Some(preset) => {
                    let name = preset.name.clone();
                    match self.settings.save_preset(&name) {
                        Ok(()) => self.notify(format!("Saved {name}")),
                        Err(error) => self.notify(error.to_string()),
                    }
                }
                None => self.dialog = Some(Dialog::save_as(&self.settings)),
            },
            PresetAction::SaveAs => self.dialog = Some(Dialog::save_as(&self.settings)),
            PresetAction::Rename => self.dialog = own.and_then(|i| Dialog::rename(&self.settings, i)),
            PresetAction::Delete => self.dialog = own.and_then(|i| Dialog::delete(&self.settings, i)),
            PresetAction::RenameAt(index) => self.dialog = Dialog::rename(&self.settings, index),
            PresetAction::DeleteAt(index) => self.dialog = Dialog::delete(&self.settings, index),
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
            Ok((name, imported)) => match self.settings.import_preset(&name, &imported.curve) {
                Ok(name) => {
                    self.selected = None;
                    match imported.skipped {
                        0 => format!("Imported “{name}”"),
                        1 => format!("Imported “{name}”, 1 filter skipped"),
                        n => format!("Imported “{name}”, {n} filters skipped"),
                    }
                }
                Err(error) => format!("Could not import “{name}”: {error}"),
            },
            Err(error) => error,
        };
        self.notify(notice);
    }

    /// Follows up on a preset sheet that was confirmed.
    fn dialog_done(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Saved(name) => self.notify(format!("Saved {name}")),
            Outcome::Renamed { old, new } => self.history.rekey(&old, Some(&new)),
            Outcome::Deleted(name) => {
                self.history.rekey(&name, None);
                self.notify(format!("Deleted {name}"));
            }
        }
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
            self.notify(format!("Could not change Open at Login: {error}"));
        }
        self.open_at_login = login::enabled();
        // macOS may hold a new login item until the user allows it.
        if self.open_at_login && login::needs_approval() {
            self.notify_for("To open EQ at login, allow it under Login Items in System Settings", WARNING_HOLD);
            login::open_settings();
        }
    }

    /// Runs what was chosen from the menus since the last call. A drag in the window can
    /// span this call, and `gesture_active` keeps it from ending the drag's undo step.
    fn run_commands(&mut self, ctx: &egui::Context, gesture_active: bool) {
        let mut commands = self.menu.as_mut().map(AppMenu::take_commands).unwrap_or_default();
        if std::mem::take(&mut self.undo_requested) {
            commands.insert(0, Command::Undo);
        }
        if commands.is_empty() {
            // Nothing can have changed, but a gesture that ended still ends its undo step.
            if !gesture_active {
                self.history.commit();
            }
            self.push_changes();
            return;
        }
        // Undo and redo replace the state outright, so they are not themselves recorded.
        for command in &commands {
            match command {
                Command::Undo => self.undo(),
                Command::Redo if self.history.redo(&mut self.settings) => {
                    self.selected = None;
                    self.mark_changed();
                }
                _ => {}
            }
        }

        let before = self.settings.clone();
        for command in commands {
            match command {
                Command::ToggleBypass => self.settings.eq.bypass = !self.settings.eq.bypass,
                Command::AddBand => {
                    if self.settings.eq.mode == Mode::Parametric {
                        self.settings.eq.add_band(&mut self.selected);
                    }
                }
                Command::DeleteBand => {
                    if let Some(index) = self.selected.take()
                        && index < self.settings.eq.bands.len()
                    {
                        self.settings.eq.bands.remove(index);
                        self.undoable = Some(format!("Band {} deleted", index + 1));
                    }
                }
                Command::RemoveAllBands => {
                    if self.settings.eq.mode == Mode::Parametric && !self.settings.eq.bands.is_empty() {
                        self.settings.eq.bands.clear();
                        self.selected = None;
                        self.undoable = Some("All bands removed".into());
                    }
                }
                Command::Reset => {
                    if self.settings.eq.can_reset() {
                        self.settings.eq.reset();
                        self.selected = None;
                        self.undoable = Some(match self.settings.eq.mode {
                            Mode::Parametric => "Bands and preamp reset".into(),
                            Mode::Graphic => "Sliders and preamp reset".into(),
                        });
                    }
                }
                Command::SetMode(mode) => {
                    if self.settings.eq.mode != mode {
                        self.settings.eq.mode = mode;
                        self.selected = None;
                    }
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

    fn undo(&mut self) {
        if self.history.undo(&mut self.settings) {
            self.selected = None;
            self.mark_changed();
        }
    }

    /// Call with the settings from before a round of edits, to save, send and record them.
    fn finish_edits(&mut self, before: &Settings, gesture_active: bool) {
        if self.settings != *before {
            self.settings.eq.clamp();
            self.mark_changed();
        }
        self.history.observe(before, &self.settings, gesture_active);
        if let Some(text) = self.undoable.take() {
            // A one-off action is a step of its own, which the message's Undo undoes. An
            // action that changed nothing undoable, such as only switching mode, offers none.
            let serial = self.history.serial();
            self.history.commit();
            let undo = (self.history.serial() != serial).then(|| self.history.serial());
            self.notice = Some(Notice {
                text,
                shown: Instant::now(),
                hold: NOTICE_HOLD,
                undo,
            });
        }
        self.push_changes();
    }

    fn sync_menu(&mut self, ctx: &egui::Context) {
        let Some(menu) = &mut self.menu else {
            return;
        };
        // While a sheet is open or a value is being typed, ⌘Z belongs to the text field.
        let no_sheet = self.dialog.is_none() && !ctx.egui_wants_keyboard_input();
        let eq = &self.settings.eq;
        let parametric = eq.mode == Mode::Parametric;
        let own = presets::active_user_preset(&self.settings);
        menu.sync(MenuState {
            can_undo: no_sheet && self.history.can_undo(),
            can_redo: no_sheet && self.history.can_redo(),
            bypass: eq.bypass,
            mode: eq.mode,
            can_add: parametric && eq.bands.len() < MAX_BANDS,
            has_selection: parametric && self.selected.is_some(),
            can_remove_all: parametric && !eq.bands.is_empty(),
            can_reset: eq.can_reset(),
            per_device: self.settings.per_device,
            presets: self.settings.all_presets().map(|p| p.name.clone()).collect(),
            active_preset: self.settings.active_index(),
            can_save_preset: own.is_some() && self.settings.is_modified(),
            can_change_preset: own.is_some(),
            open_at_login: self.open_at_login,
        });
    }

    /// Writes the settings if they have changed. A failed save is retried later, and
    /// reported once rather than on every attempt.
    fn save(&mut self) {
        if self.save_at.is_none() {
            return;
        }
        match self.settings.save() {
            Ok(()) => {
                self.save_at = None;
                if std::mem::take(&mut self.save_failed) {
                    self.notify("Settings saved");
                }
            }
            Err(error) => {
                eprintln!("could not save settings: {error}");
                self.save_at = Some(Instant::now() + SAVE_RETRY);
                if !std::mem::replace(&mut self.save_failed, true) {
                    self.notify_for(format!("Could not save settings: {error}"), WARNING_HOLD);
                }
            }
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
        self.sync_menu(ctx);
        // A moving meter needs a steady frame rate. Otherwise only the device poll and the
        // timed messages need waking for: input and menu choices wake the app themselves.
        let meter_moving = !self.hidden && self.output_db > METER_FLOOR_DB;
        ctx.request_repaint_after(if meter_moving { METER_FRAME } else { DEVICE_POLL });
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
        // Shown once the window is first seen, which at login may be much later.
        if let Some(text) = self.startup_warning.take() {
            self.notify_for(text, WARNING_HOLD);
        }

        let before = self.settings.clone();
        // A file dropped while a sheet is open would change the presets under it.
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        if self.dialog.is_none() {
            for file in &dropped {
                self.import(file.path());
            }
        }

        let view = View {
            level: Level {
                output_db: self.output_db,
                clipped: self.clipped_at.is_some_and(|t| t.elapsed() < CLIP_HOLD),
            },
            status: self.status(),
            // Either running without a sound, or still waiting to start since EQ opened,
            // which is also what a refused audio capture prompt looks like.
            silent: (self.running.is_some()
                && self.waiting_for_audio.is_some_and(|t| t.elapsed() > SILENCE_HINT_AFTER))
                || (!self.heard_audio
                    && self.starting.as_ref().is_some_and(|s| s.since.elapsed() > SILENCE_HINT_AFTER)),
            sample_rate: self.sample_rate(),
            // While a sheet is open, the keyboard belongs to it.
            keys: self.dialog.is_none(),
        };
        let (requests, _) = ui::window(ui, &mut self.settings, &mut self.selected, &view);

        if let Some(action) = requests.preset {
            self.preset_action(action);
        }
        if let Some(text) = requests.notice {
            self.notify(text);
        }
        if let Some(text) = requests.undoable {
            self.undoable = Some(text);
        }
        if requests.undo {
            self.undo_requested = true;
            ui.ctx().request_repaint();
        }
        if let Some(dialog) = &mut self.dialog
            && let Some(outcome) = dialog.show(ui.ctx(), &mut self.settings)
        {
            self.dialog = None;
            if let Some(outcome) = outcome {
                self.dialog_done(outcome);
            }
        }

        let pointer_down = ui.input(|i| i.pointer.any_down());
        self.finish_edits(&before, pointer_down);
        self.sync_menu(ui.ctx());
    }

    fn on_exit(&mut self) {
        self.save();
        // Stop processing before the window goes away so system audio is restored cleanly.
        self.running = None;
    }
}
