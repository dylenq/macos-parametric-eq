use crate::audio::devices;
use crate::audio::engine::Engine;
use crate::dsp::chain::ChainParams;
use crate::state::EqState;
use crate::ui::controls::{self, Level};
use crate::ui::curve;
use eframe::egui::{self, CentralPanel, Frame, Panel, RichText};
use std::time::{Duration, Instant};

const DEVICE_POLL: Duration = Duration::from_millis(500);
const SAVE_DELAY: Duration = Duration::from_secs(1);
const CLIP_HOLD: Duration = Duration::from_secs(2);
const SILENCE_HINT_AFTER: Duration = Duration::from_secs(8);
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
    state: EqState,
    selected: Option<usize>,
    running: Option<Running>,
    error: Option<String>,
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
}

impl App {
    pub fn new() -> Self {
        App {
            state: EqState::load(),
            selected: None,
            running: None,
            error: None,
            unsent: false,
            unsaved_since: None,
            last_device_poll: None,
            output_db: -120.0,
            clipped_at: None,
            waiting_for_audio: None,
            heard_audio: false,
            last_frame: Instant::now(),
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
        if self.last_device_poll.is_some_and(|t| t.elapsed() < DEVICE_POLL) {
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

        // The old tap must be gone before a new one mutes the system again.
        self.running = None;
        let state = &self.state;
        match Engine::start(device, |rate| ChainParams::from_state(state, rate)) {
            Ok(engine) => {
                self.running = Some(Running {
                    engine,
                    device_rate,
                    device_name: devices::device_name(device).unwrap_or_else(|| "Output".into()),
                });
                self.error = None;
                self.unsent = false;
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
        match (&self.running, &self.error) {
            (Some(r), _) => format!("{} · {:.1} kHz", r.device_name, r.engine.sample_rate / 1000.0),
            (None, Some(error)) => error.clone(),
            (None, None) => "Starting…".into(),
        }
    }

    fn push_changes(&mut self) {
        if self.unsent
            && let Some(running) = &mut self.running
        {
            let params = ChainParams::from_state(&self.state, running.engine.sample_rate);
            self.unsent = !running.engine.send(params);
        }
        if self.unsaved_since.is_some_and(|t| t.elapsed() >= SAVE_DELAY) {
            self.save();
        }
    }

    fn save(&mut self) {
        if self.unsaved_since.take().is_some()
            && let Err(error) = self.state.save()
        {
            eprintln!("could not save settings: {error}");
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let dt = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        self.follow_output_device();
        self.read_meters(dt);

        let before = self.state.clone();
        let level = Level {
            output_db: self.output_db,
            clipped: self.clipped_at.is_some_and(|t| t.elapsed() < CLIP_HOLD),
        };
        let status = self.status();
        let silent = self.running.is_some()
            && self.waiting_for_audio.is_some_and(|t| t.elapsed() > SILENCE_HINT_AFTER);
        let sample_rate = self.sample_rate();

        Panel::top("top")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(10.0))
            .show(ui, |ui| {
                controls::top_bar(ui, &mut self.state, &level, &status);
                if silent {
                    ui.label(
                        RichText::new(
                            "No audio detected. If something is playing, allow EQ under System \
                             Settings > Privacy & Security > Screen & System Audio Recording.",
                        )
                        .weak()
                        .small(),
                    );
                }
            });
        Panel::bottom("bands")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(10.0))
            .resizable(true)
            .default_size(230.0)
            .min_size(80.0)
            .show(ui, |ui| {
                controls::band_list(ui, &mut self.state, &mut self.selected);
            });
        CentralPanel::default().show(ui, |ui| {
            curve::show(ui, &mut self.state, &mut self.selected, sample_rate);
        });

        if self.state != before {
            self.state.bands.iter_mut().for_each(|b| b.clamp());
            self.unsent = true;
            self.unsaved_since = Some(Instant::now());
        }
        self.push_changes();
        // Keeps the meter moving and the device poll running while idle.
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    fn on_exit(&mut self) {
        self.save();
        // Stop processing before the window goes away so system audio is restored cleanly.
        self.running = None;
    }
}
