mod app;
mod appkit;
mod audio;
mod dsp;
mod history;
mod import;
mod login;
mod menu;
mod state;
mod tray;
mod ui;

use audio::devices;
use eframe::egui;
use audio::engine::Engine;
use dsp::chain::ChainParams;
use state::Settings;
use std::fs::{File, TryLockError};
use std::time::Duration;

const USAGE: &str = "\
Usage: eq [option]

With no option, opens the equalizer window.

Options:
  --headless [seconds]  Equalize without a window for a while (10 s by default),
                        printing the levels once a second
  --status              Print the output device, its format and the settings file,
                        then exit; status 1 means EQ could not run as things are
  --version             Print the version
  --help                Print this message
";

/// Runs the engine without a window and prints levels once a second: `eq --headless [seconds]`.
fn headless(seconds: u64) -> Result<(), String> {
    let _lock = lock_instance()?;
    let (settings, problem) = Settings::load();
    if let Some(problem) = problem {
        eprintln!("warning: {problem}");
    }
    let state = settings.eq;
    let device = devices::default_output_device().ok_or("no default output device")?;
    let engine = Engine::start(device, |rate| ChainParams::from_state(&state, rate))?;
    println!(
        "processing audio for \"{}\" at {} Hz",
        devices::device_name(device).unwrap_or_default(),
        engine.sample_rate
    );
    for _ in 0..seconds {
        std::thread::sleep(Duration::from_secs(1));
        if engine.meters.failed() {
            return Err("audio processing stopped unexpectedly".into());
        }
        let (input, output) = engine.meters.take();
        println!("in {:6.1} dB   out {:6.1} dB", to_db(input), to_db(output));
    }
    Ok(())
}

fn to_db(linear: f32) -> f32 {
    20.0 * linear.max(1e-6).log10()
}

/// Reports what EQ would work with, for setup checks and bug reports. It only reads: no
/// audio is started and no file is changed. Fails if EQ could not run as things are.
fn status() -> Result<(), String> {
    println!("EQ {}", env!("CARGO_PKG_VERSION"));
    let mut problems = Vec::new();

    match Settings::path() {
        Some(path) => {
            let readable = match std::fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                    Ok(settings) => format!("{} presets", settings.presets.len()),
                    Err(error) => {
                        problems.push("the settings file can't be read by this version");
                        format!("unreadable: {error}")
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => "not saved yet".into(),
                Err(error) => {
                    problems.push("the settings file can't be read");
                    error.to_string()
                }
            };
            println!("settings: {} ({readable})", path.display());
        }
        None => println!("settings: no Application Support folder"),
    }
    // Only an existing lock file is looked at, so this creates nothing.
    let lock = Settings::dir().and_then(|dir| File::open(dir.join(LOCK_FILE)).ok());
    let running = lock.is_some_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)));
    println!("running: {}", if running { "yes" } else { "no" });

    let Some(device) = devices::default_output_device() else {
        return Err("no default output device".into());
    };
    let name = devices::device_name(device).unwrap_or_else(|| "unnamed".into());
    let uid = devices::device_uid(device).map(|u| u.to_string()).unwrap_or_default();
    println!("output: {name} [{uid}]");
    match devices::sample_rate(device) {
        Some(rate) => println!("sample rate: {rate} Hz"),
        None => problems.push("the output's sample rate can't be read"),
    }
    let formats = devices::output_formats(device);
    let streams: Vec<usize> = formats.iter().map(|f| f.mChannelsPerFrame as usize).collect();
    let channels: usize = streams.iter().sum();
    let float = formats.iter().all(devices::is_float32);
    println!(
        "format: {} stream(s), {channels} channel(s), {}",
        formats.len(),
        if float { "32-bit float" } else { "not 32-bit float" }
    );
    if !float {
        problems.push("the output's format can't be processed");
    }
    let (left, right) = devices::speaker_channels(devices::stereo_channels(device), channels);
    match right {
        Some(right) => println!("speakers: channels {} and {}", left + 1, right + 1),
        None => println!("speakers: channel {} (mono)", left + 1),
    }
    // The stream EQ taps, numbered from 1 here.
    let stream = devices::stream_of_channel(&streams, left).unwrap_or(0);
    println!("tapped: stream {} of {}", stream + 1, streams.len().max(1));
    match problems.is_empty() {
        true => Ok(()),
        false => Err(problems.join("; ")),
    }
}

/// In the settings folder, locked by the running EQ.
const LOCK_FILE: &str = "eq.lock";

/// Locks the file that marks EQ as running, for as long as the returned file is open.
fn instance_lock() -> Result<Option<File>, ()> {
    let Some(dir) = Settings::dir() else {
        return Ok(None);
    };
    let file = std::fs::create_dir_all(&dir).and_then(|()| {
        File::options().create(true).truncate(false).write(true).open(dir.join(LOCK_FILE))
    });
    match file.map(|file| (file.try_lock(), file)) {
        Ok((Ok(()), file)) => Ok(Some(file)),
        Ok((Err(TryLockError::WouldBlock), _)) => Err(()),
        // A lock that cannot be taken at all is no reason not to run.
        Ok((Err(TryLockError::Error(_)), _)) | Err(_) => Ok(None),
    }
}

/// Keeps a second EQ from starting. Each tap leaves out only its own process, so two would
/// capture, and mute, each other's output, and both would write the settings.
fn lock_instance() -> Result<Option<File>, String> {
    instance_lock().map_err(|()| "EQ is already running. Quit it first (EQ > Quit EQ).".into())
}

fn exit_with(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Anything else, such as the -psn or -NS arguments macOS may pass, opens the window.
    match args.get(1).map(String::as_str) {
        Some("--version" | "-V") => println!("eq {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => print!("{USAGE}"),
        Some("--status") => exit_with(status()),
        Some("--headless") => {
            let seconds = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            exit_with(headless(seconds));
        }
        _ => window(),
    }
}

fn window() {
    let _lock = match lock_instance() {
        Ok(lock) => lock,
        Err(error) => {
            // Opened again, say from another copy of the app: bring the running one forward.
            appkit::activate_other_instance();
            eprintln!("{error}");
            return;
        }
    };
    let (settings, problem) = Settings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("EQ")
            .with_inner_size([920.0, 640.0])
            .with_min_inner_size([640.0, 440.0])
            // Content runs under a transparent title bar so the window reads as one surface.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            // Closed to the menu bar last time, so it starts there.
            .with_visible(!settings.window_hidden),
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "EQ",
        options,
        Box::new(|cc| {
            ui::theme::install(&cc.egui_ctx);
            Ok(Box::new(app::App::new(&cc.egui_ctx, settings, problem)))
        }),
    ) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_turns_a_second_instance_away() {
        let path = std::env::temp_dir().join(format!("eq-test-{}.lock", std::process::id()));
        let open = || File::options().create(true).truncate(false).write(true).open(&path).unwrap();
        let first = open();
        first.try_lock().unwrap();
        // flock locks belong to the open file, so a second open in this process conflicts
        // just as another process would.
        assert!(matches!(open().try_lock(), Err(TryLockError::WouldBlock)));
        drop(first);
        assert!(open().try_lock().is_ok());
        std::fs::remove_file(&path).unwrap();
    }
}
