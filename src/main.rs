mod app;
mod audio;
mod dsp;
mod state;
mod ui;

use audio::devices;
use eframe::egui;
use audio::engine::Engine;
use dsp::chain::ChainParams;
use state::EqState;
use std::time::Duration;

/// Runs the engine without a window and prints levels once a second: `eq --headless [seconds]`.
fn headless(seconds: u64) -> Result<(), String> {
    let state = EqState::load();
    let device = devices::default_output_device().ok_or("no default output device")?;
    let engine = Engine::start(device, |rate| ChainParams::from_state(&state, rate))?;
    println!(
        "processing audio for \"{}\" at {} Hz",
        devices::device_name(device).unwrap_or_default(),
        engine.sample_rate
    );
    for _ in 0..seconds {
        std::thread::sleep(Duration::from_secs(1));
        let (input, output) = engine.meters.take();
        println!("in {:6.1} dB   out {:6.1} dB", to_db(input), to_db(output));
    }
    Ok(())
}

fn to_db(linear: f32) -> f32 {
    20.0 * linear.max(1e-6).log10()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--headless") {
        let seconds = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
        if let Err(error) = headless(seconds) {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        return;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("EQ")
            .with_inner_size([920.0, 620.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "EQ",
        options,
        Box::new(|_| Ok(Box::new(app::App::new()))),
    ) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
