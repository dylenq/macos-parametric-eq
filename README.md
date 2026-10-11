# EQ

A system-wide parametric equalizer for macOS. It equalizes everything your Mac plays
(music apps, browsers, video calls) without installing an audio driver.

- Drag band nodes on the frequency-response curve to set frequency and gain
- Scroll over a node to change Q, right-click it to change filter type or delete it
- Double-click empty space to add a band (up to 16)
- Peak, low/high shelf and low/high pass filters
- Preamp, bypass for A/B comparison, and an output meter with a clip indicator
- Follows the default output device when you switch speakers or headphones

Requires macOS 14.2 or later. Runs on Apple silicon and Intel.

## Install

1. Download `EQ.zip` from the Releases page and unzip it.
2. Move `EQ.app` to Applications and open it.
3. The app is not notarized by Apple, so macOS blocks the first launch. Open
   **System Settings > Privacy & Security**, scroll down, and click **Open Anyway**.
4. Allow system audio capture when macOS asks. If you hear nothing, enable EQ under
   **System Settings > Privacy & Security > Screen & System Audio Recording**.

Quitting the app restores normal audio immediately.

## How it works

EQ creates a Core Audio process tap of all system audio except its own, muting the
original at the hardware while it is being read. A private aggregate device pairs the tap
with your real output device, and a single IOProc runs the audio through a cascade of
biquad filters (RBJ cookbook designs) on its way to the speakers. Parameter changes are
passed to the audio thread through a lock-free queue and smoothed to avoid clicks.

Settings are saved to `~/Library/Application Support/eq/state.json`.

## Build from source

You need Xcode's command line tools and Rust (via [rustup](https://rustup.rs)).

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
./scripts/bundle.sh
open target/EQ.app
```

Run the app from the bundle rather than with `cargo run`: macOS only grants audio
capture to an app bundle. `./scripts/bundle.sh --zip` also writes `target/EQ.zip`.

```sh
cargo test                                         # DSP unit tests
target/EQ.app/Contents/MacOS/eq --headless 10      # no window, prints levels
```

## License

GPL-3.0. See [LICENSE](LICENSE).
