use crate::dsp::graphic;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MAX_BANDS: usize = 16;
pub const MIN_FREQ: f32 = 20.0;
pub const MAX_FREQ: f32 = 20_000.0;
pub const MIN_Q: f32 = 0.1;
pub const MAX_Q: f32 = 18.0;
pub const MAX_GAIN_DB: f32 = 24.0;
/// Range of a graphic slider and of the bass and treble controls.
pub const SLIDER_MAX_DB: f32 = 12.0;
/// Within this much of centre, balance is centre.
const BALANCE_SNAP: f32 = 0.01;
pub const MAX_PRESET_NAME: usize = 40;
pub const MAX_USER_PRESETS: usize = 100;
/// Where "Add Band" puts a new band: the first of these with no band near it.
const ADD_FREQS: [f32; 8] = [1_000.0, 250.0, 4_000.0, 100.0, 10_000.0, 60.0, 2_000.0, 8_000.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterKind {
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

impl FilterKind {
    pub const ALL: [FilterKind; 5] = [
        FilterKind::Peak,
        FilterKind::LowShelf,
        FilterKind::HighShelf,
        FilterKind::LowPass,
        FilterKind::HighPass,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterKind::Peak => "Peak",
            FilterKind::LowShelf => "Low Shelf",
            FilterKind::HighShelf => "High Shelf",
            FilterKind::LowPass => "Low Pass",
            FilterKind::HighPass => "High Pass",
        }
    }

    /// Pass filters have no gain parameter.
    pub fn has_gain(self) -> bool {
        !matches!(self, FilterKind::LowPass | FilterKind::HighPass)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub kind: FilterKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
    pub enabled: bool,
}

impl Band {
    pub fn new(kind: FilterKind, freq: f32) -> Self {
        Band {
            kind,
            freq,
            gain_db: 0.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
            enabled: true,
        }
    }

    pub fn clamp(&mut self) {
        self.freq = clamp_finite(self.freq, MIN_FREQ, MAX_FREQ, 1_000.0);
        self.q = clamp_finite(self.q, MIN_Q, MAX_Q, std::f32::consts::FRAC_1_SQRT_2);
        self.gain_db = clamp_finite(self.gain_db, -MAX_GAIN_DB, MAX_GAIN_DB, 0.0);
    }
}

/// `value` clamped into `min..=max`, or `fallback` if it is not a number at all. `f32::clamp`
/// keeps NaN, which would reach the audio as NaN samples and the file as `null`.
fn clamp_finite(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() { value.clamp(min, max) } else { fallback }
}

/// Which editor shapes the sound. Each keeps its own settings while the other is in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Parametric,
    Graphic,
}

/// A graphic equalizer: one slider per fixed frequency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graphic {
    pub band_count: u32,
    pub gains_db: Vec<f32>,
}

impl Default for Graphic {
    fn default() -> Self {
        Graphic {
            band_count: 10,
            gains_db: vec![0.0; 10],
        }
    }
}

impl Graphic {
    /// Changes the number of sliders, keeping the shape of the curve.
    pub fn set_band_count(&mut self, count: u32) {
        if count != self.band_count && graphic::centres(count).is_some() {
            self.gains_db = graphic::resample(self.band_count, &self.gains_db, count);
            self.band_count = count;
        }
    }

    pub fn centres(&self) -> &'static [f32] {
        graphic::centres(self.band_count).unwrap_or_default()
    }

    fn sanitize(&mut self) {
        let valid = graphic::centres(self.band_count)
            .is_some_and(|c| c.len() == self.gains_db.len())
            && self.gains_db.iter().all(|g| g.is_finite());
        if !valid {
            *self = Graphic::default();
        }
        for gain in &mut self.gains_db {
            *gain = gain.clamp(-SLIDER_MAX_DB, SLIDER_MAX_DB);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqState {
    #[serde(default)]
    pub mode: Mode,
    pub bands: Vec<Band>,
    #[serde(default)]
    pub graphic: Graphic,
    pub preamp_db: f32,
    /// Low shelf at 100 Hz, applied in both modes.
    #[serde(default)]
    pub bass_db: f32,
    /// High shelf at 10 kHz, applied in both modes.
    #[serde(default)]
    pub treble_db: f32,
    /// -1 is fully left, 1 fully right. Only the opposite channel is turned down.
    #[serde(default)]
    pub balance: f32,
    pub bypass: bool,
    /// Whether a limiter holds the output under full scale instead of letting it clip.
    #[serde(default = "enabled")]
    pub clip_protection: bool,
}

fn default_bands() -> Vec<Band> {
    let mut bands = vec![Band::new(FilterKind::LowShelf, 80.0)];
    for freq in [200.0, 500.0, 1_000.0, 2_500.0, 6_000.0] {
        let mut band = Band::new(FilterKind::Peak, freq);
        band.q = 1.0;
        bands.push(band);
    }
    bands.push(Band::new(FilterKind::HighShelf, 10_000.0));
    bands
}

impl Default for EqState {
    fn default() -> Self {
        EqState {
            mode: Mode::Parametric,
            bands: default_bands(),
            graphic: Graphic::default(),
            preamp_db: 0.0,
            bass_db: 0.0,
            treble_db: 0.0,
            balance: 0.0,
            bypass: false,
            clip_protection: true,
        }
    }
}

/// Snaps a balance to centre when it is within a hair of it.
pub fn snap_balance(balance: f32) -> f32 {
    let balance = clamp_finite(balance, -1.0, 1.0, 0.0);
    if balance.abs() <= BALANCE_SNAP { 0.0 } else { balance }
}

impl EqState {
    /// Adds a flat peak band where there is room for one and selects it, if the band list
    /// is not full.
    pub fn add_band(&mut self, selected: &mut Option<usize>) {
        if self.bands.len() < MAX_BANDS {
            let near = |freq: f32| {
                self.bands
                    .iter()
                    .any(|b| b.enabled && (freq / b.freq).log2().abs() <= 1.0 / 3.0)
            };
            let freq = ADD_FREQS.into_iter().find(|&f| !near(f)).unwrap_or(1_000.0);
            let mut band = Band::new(FilterKind::Peak, freq);
            band.q = 1.0;
            self.bands.push(band);
            *selected = Some(self.bands.len() - 1);
        }
    }

    /// Flattens the editor in use and the preamp. Bass, treble, balance, the other mode
    /// and the switches are left as they are.
    pub fn reset(&mut self) {
        match self.mode {
            Mode::Parametric => self.bands = default_bands(),
            Mode::Graphic => self.graphic.gains_db.fill(0.0),
        }
        self.preamp_db = 0.0;
    }

    /// Whether `reset` would change anything.
    pub fn can_reset(&self) -> bool {
        let editor = match self.mode {
            Mode::Parametric => self.bands != default_bands(),
            Mode::Graphic => self.graphic.gains_db.iter().any(|&g| g != 0.0),
        };
        editor || self.preamp_db != 0.0
    }

    /// True when only bass, treble or balance are left to undo with their own sliders.
    pub fn tone_set(&self) -> bool {
        self.bass_db != 0.0 || self.treble_db != 0.0 || self.balance != 0.0
    }

    pub fn curve(&self) -> Curve {
        Curve {
            mode: self.mode,
            bands: self.bands.clone(),
            graphic: self.graphic.clone(),
            preamp_db: self.preamp_db,
            bass_db: self.bass_db,
            treble_db: self.treble_db,
            balance: self.balance,
        }
    }

    pub fn set_curve(&mut self, curve: &Curve) {
        self.mode = curve.mode;
        self.bands = curve.bands.clone();
        self.graphic = curve.graphic.clone();
        self.preamp_db = curve.preamp_db;
        self.bass_db = curve.bass_db;
        self.treble_db = curve.treble_db;
        self.balance = curve.balance;
    }

    /// Clamps every value into range, after an edit or a load.
    pub fn clamp(&mut self) {
        self.bands.iter_mut().for_each(Band::clamp);
        self.graphic.sanitize();
        self.preamp_db = clamp_finite(self.preamp_db, -MAX_GAIN_DB, MAX_GAIN_DB, 0.0);
        self.bass_db = clamp_finite(self.bass_db, -SLIDER_MAX_DB, SLIDER_MAX_DB, 0.0);
        self.treble_db = clamp_finite(self.treble_db, -SLIDER_MAX_DB, SLIDER_MAX_DB, 0.0);
        self.balance = snap_balance(self.balance);
    }
}

/// The part of an [`EqState`] that presets and devices remember: the whole sound, but not
/// bypass or clip protection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    #[serde(default)]
    pub mode: Mode,
    pub bands: Vec<Band>,
    #[serde(default)]
    pub graphic: Graphic,
    pub preamp_db: f32,
    #[serde(default)]
    pub bass_db: f32,
    #[serde(default)]
    pub treble_db: f32,
    #[serde(default)]
    pub balance: f32,
}

/// Values closer than this count as the same, so a value that went through a text field
/// does not mark a preset as edited.
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4
}

fn near_freq(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
}

impl Curve {
    fn sanitize(&mut self) {
        let mut state = EqState::default();
        self.bands.truncate(MAX_BANDS);
        state.set_curve(self);
        state.clamp();
        *self = state.curve();
    }

    /// Equal within a rounding error, comparing both editors whichever is in use.
    pub fn same_as(&self, other: &Curve) -> bool {
        let bands = self.bands.len() == other.bands.len()
            && self.bands.iter().zip(&other.bands).all(|(a, b)| {
                a.kind == b.kind
                    && a.enabled == b.enabled
                    && near_freq(a.freq, b.freq)
                    && near(a.gain_db, b.gain_db)
                    && near(a.q, b.q)
            });
        let graphic = self.graphic.band_count == other.graphic.band_count
            && self.graphic.gains_db.len() == other.graphic.gains_db.len()
            && self.graphic.gains_db.iter().zip(&other.graphic.gains_db).all(|(a, b)| near(*a, *b));
        self.mode == other.mode
            && bands
            && graphic
            && near(self.preamp_db, other.preamp_db)
            && near(self.bass_db, other.bass_db)
            && near(self.treble_db, other.treble_db)
            && near(self.balance, other.balance)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(flatten)]
    pub curve: Curve,
}

/// Presets that come with the app. They cannot be changed, and their names are taken.
pub fn builtins() -> &'static [Preset] {
    static BUILTINS: std::sync::OnceLock<Vec<Preset>> = std::sync::OnceLock::new();
    BUILTINS.get_or_init(|| {
        [
            ("Flat", [0.0; 10], 0.0),
            ("Bass", [6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], -6.0),
            ("Treble", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0], -6.0),
            ("Vocal", [0.0, -2.0, -1.0, 0.0, 2.0, 3.0, 3.0, 2.0, 0.0, 0.0], -3.0),
        ]
        .into_iter()
        .map(|(name, gains, preamp_db)| Preset {
            name: name.into(),
            curve: Curve {
                mode: Mode::Graphic,
                bands: default_bands(),
                graphic: Graphic {
                    band_count: 10,
                    gains_db: gains.to_vec(),
                },
                preamp_db,
                ..Curve::default()
            },
        })
        .collect()
    })
}

/// Why a preset could not be saved, renamed or deleted, in words for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetError {
    Blank,
    Invalid,
    Taken,
    TooMany,
    BuiltIn,
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PresetError::Blank => "Enter a name.",
            PresetError::Invalid => "Names must be 1 to 40 characters.",
            PresetError::Taken => "A preset with that name already exists.",
            PresetError::TooMany => "You can keep up to 100 presets. Delete one to save another.",
            PresetError::BuiltIn => "Built-in presets can't be changed.",
        })
    }
}

/// Names are compared ignoring case and surrounding space.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// `base`, or `base` with the first number from `n` up that makes it a name `taken` does not
/// know, shortened to keep within the length limit. Leaves `n` past any number it used.
fn unique_name(base: &str, n: &mut u32, taken: impl Fn(&str) -> bool) -> String {
    let mut name = base.to_owned();
    while taken(&name) {
        let suffix = format!(" {n}");
        let room = MAX_PRESET_NAME - suffix.len();
        name = format!("{}{suffix}", base.chars().take(room).collect::<String>().trim_end());
        *n += 1;
    }
    name
}

/// Checks a name typed for a preset and returns it trimmed.
pub fn check_name(name: &str) -> Result<&str, PresetError> {
    let name = name.trim();
    if name.is_empty() {
        Err(PresetError::Blank)
    } else if name.chars().count() > MAX_PRESET_NAME || name.chars().any(char::is_control) {
        Err(PresetError::Invalid)
    } else {
        Ok(name)
    }
}

/// What an output device sounded like when it was last in use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct DeviceEq {
    #[serde(flatten)]
    curve: Curve,
    #[serde(default)]
    active_preset: Option<String>,
}

fn enabled() -> bool {
    true
}

/// Everything that is saved: the equalizer as it is now, the preset library, and the
/// curve each output device was left with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Flattened so a file written before presets existed still loads.
    #[serde(flatten)]
    pub eq: EqState,
    /// The user's own presets. The built-in ones come before them wherever presets are
    /// listed, so a preset index counts the built-ins first.
    #[serde(default)]
    pub presets: Vec<Preset>,
    /// The preset `eq` was last loaded from or saved to, by name.
    #[serde(default)]
    pub active_preset: Option<String>,
    /// Whether each output device keeps its own curve.
    #[serde(default = "enabled")]
    pub per_device: bool,
    /// UID of the output device `eq` currently belongs to.
    #[serde(default)]
    current_device: Option<String>,
    #[serde(default)]
    devices: BTreeMap<String, DeviceEq>,
    /// Whether the window was closed, leaving EQ in the menu bar, when it last ran.
    #[serde(default)]
    pub window_hidden: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            eq: EqState::default(),
            presets: Vec::new(),
            active_preset: None,
            per_device: true,
            current_device: None,
            devices: BTreeMap::new(),
            window_hidden: false,
        }
    }
}

impl Settings {
    /// Every preset, built-in ones first.
    pub fn all_presets(&self) -> impl Iterator<Item = &Preset> {
        builtins().iter().chain(&self.presets)
    }

    pub fn preset(&self, index: usize) -> Option<&Preset> {
        self.all_presets().nth(index)
    }

    pub fn is_builtin(index: usize) -> bool {
        index < builtins().len()
    }

    /// Index into `presets` of the user preset at a combined index.
    fn user_index(&self, index: usize) -> Result<usize, PresetError> {
        if Self::is_builtin(index) {
            return Err(PresetError::BuiltIn);
        }
        let user = index - builtins().len();
        if user < self.presets.len() { Ok(user) } else { Err(PresetError::Invalid) }
    }

    pub fn active_index(&self) -> Option<usize> {
        let active = self.active_preset.as_deref()?;
        self.all_presets().position(|p| p.name == active)
    }

    /// True if the equalizer has been edited since its preset was loaded or saved.
    pub fn is_modified(&self) -> bool {
        self.active_index()
            .and_then(|i| self.preset(i))
            .is_some_and(|p| !p.curve.same_as(&self.eq.curve()))
    }

    pub fn apply_preset(&mut self, index: usize) {
        if let Some(preset) = self.preset(index).cloned() {
            self.eq.set_curve(&preset.curve);
            self.active_preset = Some(preset.name);
        }
    }

    /// Saves the equalizer as it is now under `name`, replacing a user preset of that name,
    /// whatever its case.
    pub fn save_preset(&mut self, name: &str) -> Result<(), PresetError> {
        let name = check_name(name)?;
        if builtins().iter().any(|p| same_name(&p.name, name)) {
            return Err(PresetError::Taken);
        }
        let mut curve = self.eq.curve();
        curve.sanitize();
        let existing = self.presets.iter().position(|p| same_name(&p.name, name));
        let stored = match existing {
            Some(index) => {
                self.presets[index].curve = curve;
                self.presets[index].name.clone()
            }
            None if self.presets.len() >= MAX_USER_PRESETS => return Err(PresetError::TooMany),
            None => {
                self.presets.push(Preset {
                    name: name.into(),
                    curve,
                });
                name.into()
            }
        };
        self.active_preset = Some(stored);
        Ok(())
    }

    /// Loads bands and a preamp read from a file and keeps the result as a preset. Bass and
    /// treble are turned off, since the file describes the whole correction; the graphic
    /// sliders and balance are kept.
    pub fn import_preset(&mut self, name: &str, curve: &Curve) -> Result<String, PresetError> {
        let name: String = name.trim().chars().filter(|c| !c.is_control()).collect();
        let name = name.chars().take(MAX_PRESET_NAME).collect::<String>().trim().to_owned();
        let base = if name.is_empty() { "Imported".to_owned() } else { name };
        // An import never replaces a preset, which undo could not bring back: a name already
        // in use, built-in or the user's own, gets a number.
        let name = unique_name(&base, &mut 2, |name| self.all_presets().any(|p| same_name(&p.name, name)));
        let before = self.eq.clone();
        self.eq.mode = Mode::Parametric;
        self.eq.bands = curve.bands.clone();
        self.eq.preamp_db = curve.preamp_db;
        self.eq.bass_db = 0.0;
        self.eq.treble_db = 0.0;
        if let Err(error) = self.save_preset(&name) {
            self.eq = before;
            return Err(error);
        }
        Ok(self.active_preset.clone().unwrap_or(name))
    }

    /// Returns the new name, trimmed.
    pub fn rename_preset(&mut self, index: usize, name: &str) -> Result<String, PresetError> {
        let user = self.user_index(index)?;
        let name = check_name(name)?;
        let taken = self.all_presets().enumerate().any(|(i, p)| i != index && same_name(&p.name, name));
        if taken {
            return Err(PresetError::Taken);
        }
        let old = std::mem::replace(&mut self.presets[user].name, name.into());
        self.for_each_reference(|active| {
            if active.as_deref() == Some(&old) {
                *active = Some(name.into());
            }
        });
        Ok(name.into())
    }

    /// Removes a preset and returns its name. The equalizer keeps sounding the way it does.
    pub fn delete_preset(&mut self, index: usize) -> Result<String, PresetError> {
        let user = self.user_index(index)?;
        let removed = self.presets.remove(user).name;
        self.for_each_reference(|active| {
            if active.as_deref() == Some(&removed) {
                *active = None;
            }
        });
        Ok(removed)
    }

    /// Visits every place that names a preset.
    fn for_each_reference(&mut self, mut visit: impl FnMut(&mut Option<String>)) {
        visit(&mut self.active_preset);
        for device in self.devices.values_mut() {
            visit(&mut device.active_preset);
        }
    }

    /// Records that `uid` is now the output device. With `per_device` on, the outgoing
    /// device keeps the current curve and the incoming one gets back the curve it was left
    /// with, if it has been used before. Returns true if the equalizer changed.
    pub fn switch_device(&mut self, uid: &str) -> bool {
        if self.current_device.as_deref() == Some(uid) {
            return false;
        }
        let previous = self.current_device.replace(uid.into());
        if !self.per_device {
            return false;
        }
        if let Some(previous) = previous {
            self.devices.insert(
                previous,
                DeviceEq {
                    curve: self.eq.curve(),
                    active_preset: self.active_preset.clone(),
                },
            );
        }
        let Some(device) = self.devices.get(uid) else {
            return false;
        };
        let changed = device.curve != self.eq.curve();
        self.eq.set_curve(&device.curve);
        self.active_preset = device.active_preset.clone();
        changed
    }

    /// Gives every user preset a usable name, distinct from the built-in ones and from
    /// each other, and follows the renames in every reference.
    fn sanitize_names(&mut self) {
        // Compared as `same_name` does. With a set, and the next number to try for each
        // name, a file with thousands of presets of one name does not take minutes to load.
        let key = |name: &str| name.trim().to_lowercase();
        let mut taken: HashSet<String> = builtins().iter().map(|p| key(&p.name)).collect();
        let mut next: HashMap<String, u32> = HashMap::new();
        for index in 0..self.presets.len() {
            let old = self.presets[index].name.clone();
            let cleaned: String = old.chars().filter(|c| !c.is_control()).collect();
            let cleaned: String = cleaned.trim().chars().take(MAX_PRESET_NAME).collect();
            let base = match cleaned.trim() {
                "" => "Preset",
                name => name,
            };
            let n = next.entry(key(base)).or_insert(2);
            let name = unique_name(base, n, |name| taken.contains(&key(name)));
            if name != old {
                self.presets[index].name = name.clone();
                // Only the first preset that had the old name keeps its references.
                if !self.presets[..index].iter().any(|p| p.name == old) {
                    self.for_each_reference(|active| {
                        if active.as_deref() == Some(&old) {
                            *active = Some(name.clone());
                        }
                    });
                }
            }
            taken.insert(key(&name));
        }
    }

    fn sanitize(&mut self) {
        self.eq.bands.truncate(MAX_BANDS);
        self.eq.clamp();
        self.presets.iter_mut().for_each(|p| p.curve.sanitize());
        self.devices.values_mut().for_each(|d| d.curve.sanitize());
        self.sanitize_names();
    }

    /// The folder EQ keeps its files in: ~/Library/Application Support/eq.
    pub fn dir() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("eq"))
    }

    pub fn path() -> Option<PathBuf> {
        Some(Self::dir()?.join("state.json"))
    }

    /// Settings read from a file and made safe, or None if they are not settings this
    /// version can read.
    fn parse(bytes: &[u8]) -> Option<Self> {
        let mut settings: Settings = serde_json::from_slice(bytes).ok()?;
        settings.sanitize();
        Some(settings)
    }

    pub fn load() -> (Self, Option<LoadProblem>) {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => (Self::default(), None),
        }
    }

    /// Reads the settings at `path`. A file this version cannot read, which a newer version
    /// may have written, is moved aside first, so that saving cannot overwrite the presets
    /// in it.
    pub fn load_from(path: &Path) -> (Self, Option<LoadProblem>) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(error) => return (Self::default(), Some(LoadProblem::Unreadable(error))),
        };
        if let Some(settings) = Self::parse(&bytes) {
            return (settings, None);
        }
        let problem = match set_aside(path) {
            Ok(kept) => LoadProblem::SetAside(kept),
            Err(error) => LoadProblem::NotSetAside(error),
        };
        (Self::default(), Some(problem))
    }

    pub fn save(&self) -> std::io::Result<()> {
        match Self::path() {
            Some(path) => self.save_to(&path),
            None => Ok(()),
        }
    }

    /// Writes the file whole or not at all, so a crash mid-write cannot lose the settings.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        // On disk before the rename, or a power cut could leave an empty file in its place.
        file.sync_all()?;
        std::fs::rename(temp, path)
    }
}

/// Why the settings EQ started with are not the ones saved last.
#[derive(Debug)]
pub enum LoadProblem {
    /// The file could not be read at all.
    Unreadable(std::io::Error),
    /// The file was not settings this version understands. It was kept under this name.
    SetAside(PathBuf),
    /// The file was not settings this version understands, and could not be kept aside.
    NotSetAside(std::io::Error),
}

impl std::fmt::Display for LoadProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadProblem::Unreadable(error) => {
                write!(f, "Could not read the saved settings ({error}), so EQ started with defaults.")
            }
            LoadProblem::SetAside(kept) => {
                let name = kept.file_name().unwrap_or_default().to_string_lossy();
                write!(
                    f,
                    "The saved settings were from a newer version of EQ or damaged, so EQ started \
                     with defaults. The old file was kept as {name}."
                )
            }
            LoadProblem::NotSetAside(error) => write!(
                f,
                "The saved settings were from a newer version of EQ or damaged, and could not be \
                 kept aside ({error}). They will be replaced when EQ next saves."
            ),
        }
    }
}

/// Moves an unreadable settings file to the first free `<name>.unreadable[-n].json` beside it.
fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    for n in 1..100 {
        let name = match n {
            1 => format!("{stem}.unreadable.json"),
            n => format!("{stem}.unreadable-{n}.json"),
        };
        let kept = path.with_file_name(name);
        if !kept.exists() {
            std::fs::rename(path, &kept)?;
            return Ok(kept);
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "too many unreadable settings files"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_preamp(db: f32) -> Settings {
        let mut settings = Settings::default();
        settings.eq.preamp_db = db;
        settings
    }

    /// Index of the user preset at `n` in the combined list.
    fn user(n: usize) -> usize {
        builtins().len() + n
    }

    #[test]
    fn a_file_from_before_presets_still_loads() {
        let old = br#"{"bands":[{"kind":"Peak","freq":1000.0,"gain_db":3.0,"q":1.0,"enabled":true}],
            "preamp_db":-3.0,"bypass":false}"#;
        let settings = Settings::parse(old).unwrap();
        assert_eq!(settings.eq.preamp_db, -3.0);
        assert_eq!(settings.eq.bands.len(), 1);
        assert_eq!(settings.eq.mode, Mode::Parametric);
        assert_eq!((settings.eq.bass_db, settings.eq.balance), (0.0, 0.0));
        assert_eq!(settings.eq.graphic, Graphic::default());
        assert!(settings.eq.clip_protection);
        assert!(settings.presets.is_empty());
        assert!(settings.per_device);
    }

    #[test]
    fn a_file_from_0_4_with_presets_and_devices_still_loads() {
        let old = br#"{"bands":[],"preamp_db":-1.0,"bypass":true,"clip_protection":false,
            "presets":[{"name":"Warm","bands":[],"preamp_db":-4.0}],"active_preset":"Warm",
            "per_device":true,"current_device":"spk",
            "devices":{"hp":{"bands":[],"preamp_db":-2.0,"active_preset":null}}}"#;
        let mut settings = Settings::parse(old).unwrap();
        assert_eq!(settings.presets[0].curve.preamp_db, -4.0);
        assert_eq!(settings.presets[0].curve.mode, Mode::Parametric);
        assert_eq!(settings.active_index(), Some(user(0)));
        assert!(settings.switch_device("hp"));
        assert_eq!(settings.eq.preamp_db, -2.0);
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let mut settings = with_preamp(-2.0);
        settings.eq.mode = Mode::Graphic;
        settings.eq.graphic.gains_db[3] = 4.5;
        settings.eq.bass_db = 3.0;
        settings.eq.balance = -0.4;
        settings.save_preset("Warm").unwrap();
        settings.switch_device("speakers");
        settings.switch_device("headphones");
        let bytes = serde_json::to_vec(&settings).unwrap();
        assert_eq!(Settings::parse(&bytes).as_ref(), Some(&settings));
    }

    #[test]
    fn unreadable_or_out_of_range_files_are_made_safe() {
        assert_eq!(Settings::parse(b"not json"), None);
        let mut wild = with_preamp(90.0);
        wild.eq.bass_db = 40.0;
        wild.eq.balance = 3.0;
        wild.eq.graphic = Graphic {
            band_count: 7,
            gains_db: vec![1.0; 7],
        };
        wild.save_preset("Loud").unwrap();
        let settings = Settings::parse(&serde_json::to_vec(&wild).unwrap()).unwrap();
        assert_eq!(settings.eq.preamp_db, MAX_GAIN_DB);
        assert_eq!(settings.eq.bass_db, SLIDER_MAX_DB);
        assert_eq!(settings.eq.balance, 1.0);
        assert_eq!(settings.eq.graphic, Graphic::default());
        assert_eq!(settings.presets[0].curve.preamp_db, MAX_GAIN_DB);
        assert_eq!(settings.presets[0].curve.graphic, Graphic::default());
    }

    #[test]
    fn presets_save_apply_and_track_edits() {
        let mut settings = with_preamp(-4.0);
        settings.save_preset("  Warm ").unwrap();
        assert_eq!(settings.active_preset.as_deref(), Some("Warm"));
        assert!(!settings.is_modified());

        settings.eq.preamp_db = 1.0;
        assert!(settings.is_modified());
        settings.apply_preset(user(0));
        assert_eq!(settings.eq.preamp_db, -4.0);

        // Saving under the same name, in any case, replaces the preset and keeps its name.
        settings.eq.preamp_db = 2.0;
        settings.save_preset("WARM").unwrap();
        assert_eq!(settings.presets.len(), 1);
        assert_eq!(settings.presets[0].name, "Warm");
        assert_eq!(settings.presets[0].curve.preamp_db, 2.0);
    }

    #[test]
    fn edits_below_a_rounding_error_do_not_mark_a_preset_edited() {
        let mut settings = with_preamp(-4.0);
        settings.save_preset("Warm").unwrap();
        settings.eq.preamp_db += 1e-5;
        settings.eq.bands[2].freq *= 1.0 + 1e-5;
        assert!(!settings.is_modified());
        settings.eq.preamp_db += 1e-3;
        assert!(settings.is_modified());

        for edit in [
            |s: &mut Settings| s.eq.mode = Mode::Graphic,
            |s: &mut Settings| s.eq.graphic.gains_db[0] = 1.0,
            |s: &mut Settings| s.eq.bass_db = 1.0,
            |s: &mut Settings| s.eq.treble_db = 1.0,
            |s: &mut Settings| s.eq.balance = 0.5,
            |s: &mut Settings| s.eq.bands[0].enabled = false,
        ] {
            let mut edited = settings.clone();
            edited.apply_preset(user(0));
            edit(&mut edited);
            assert!(edited.is_modified());
        }
    }

    #[test]
    fn applying_a_preset_leaves_bypass_alone() {
        let mut settings = with_preamp(-4.0);
        settings.save_preset("Warm").unwrap();
        settings.eq.bypass = true;
        settings.eq.clip_protection = false;
        settings.apply_preset(user(0));
        assert!(settings.eq.bypass);
        assert!(!settings.eq.clip_protection);
    }

    #[test]
    fn built_in_presets_apply_and_cannot_be_changed() {
        let names: Vec<_> = builtins().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Flat", "Bass", "Treble", "Vocal"]);

        let mut settings = Settings::default();
        settings.apply_preset(1);
        assert_eq!(settings.eq.mode, Mode::Graphic);
        assert_eq!(settings.eq.graphic.gains_db[0], 6.0);
        assert_eq!(settings.eq.preamp_db, -6.0);
        assert_eq!(settings.active_index(), Some(1));
        assert!(!settings.is_modified());

        assert_eq!(settings.rename_preset(1, "Boom"), Err(PresetError::BuiltIn));
        assert_eq!(settings.delete_preset(0), Err(PresetError::BuiltIn));
        assert_eq!(settings.save_preset("bass"), Err(PresetError::Taken));
        assert_eq!(settings.presets.len(), 0);
    }

    #[test]
    fn preset_names_follow_the_rules() {
        let mut settings = Settings::default();
        assert_eq!(settings.save_preset("   "), Err(PresetError::Blank));
        assert_eq!(settings.save_preset(&"x".repeat(41)), Err(PresetError::Invalid));
        assert_eq!(settings.save_preset("a\u{7}b"), Err(PresetError::Invalid));
        assert!(settings.save_preset(&"é".repeat(40)).is_ok());

        for n in 1..MAX_USER_PRESETS {
            settings.save_preset(&format!("P{n}")).unwrap();
        }
        assert_eq!(settings.save_preset("One more"), Err(PresetError::TooMany));
        // Replacing one is still allowed at the limit.
        assert!(settings.save_preset("p1").is_ok());
    }

    #[test]
    fn rename_and_delete_follow_the_active_preset() {
        let mut settings = with_preamp(0.0);
        settings.save_preset("A").unwrap();
        settings.save_preset("B").unwrap();
        assert_eq!(settings.rename_preset(user(1), "a"), Err(PresetError::Taken));
        assert_eq!(settings.rename_preset(user(1), "flat"), Err(PresetError::Taken));
        assert_eq!(settings.rename_preset(user(1), " "), Err(PresetError::Blank));
        assert_eq!(settings.rename_preset(user(1), "C").as_deref(), Ok("C"));
        assert_eq!(settings.active_preset.as_deref(), Some("C"));
        // Changing only the case of a name is allowed.
        assert!(settings.rename_preset(user(1), "c").is_ok());

        assert_eq!(settings.delete_preset(user(1)).as_deref(), Ok("c"));
        assert_eq!(settings.active_preset, None);
        assert_eq!(settings.presets.len(), 1);
        assert_eq!(settings.delete_preset(user(5)), Err(PresetError::Invalid));
    }

    #[test]
    fn old_names_that_clash_are_renamed_on_load() {
        let mut old = with_preamp(0.0);
        for name in ["Flat", "warm", "Warm", ""] {
            old.presets.push(Preset {
                name: name.into(),
                curve: old.eq.curve(),
            });
        }
        old.active_preset = Some("Flat".into());
        let settings = Settings::parse(&serde_json::to_vec(&old).unwrap()).unwrap();
        let names: Vec<_> = settings.presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Flat 2", "warm", "Warm 2", "Preset"]);
        assert_eq!(settings.active_preset.as_deref(), Some("Flat 2"));
    }

    #[test]
    fn import_turns_off_tone_and_avoids_built_in_names() {
        let mut settings = with_preamp(0.0);
        settings.eq.mode = Mode::Graphic;
        settings.eq.bass_db = 4.0;
        settings.eq.balance = 0.3;
        let curve = Curve {
            bands: vec![Band::new(FilterKind::Peak, 500.0)],
            preamp_db: -3.0,
            ..Curve::default()
        };
        assert_eq!(settings.import_preset("Bass", &curve).as_deref(), Ok("Bass 2"));
        // Importing it again keeps the first import.
        assert_eq!(settings.import_preset("bass", &curve).as_deref(), Ok("bass 3"));
        assert_eq!(settings.presets.len(), 2);
        assert_eq!(settings.eq.mode, Mode::Parametric);
        assert_eq!((settings.eq.bass_db, settings.eq.balance), (0.0, 0.3));
        assert_eq!(settings.eq.bands.len(), 1);
    }

    #[test]
    fn reset_flattens_only_the_editor_in_use() {
        let mut state = EqState {
            preamp_db: -3.0,
            bass_db: 2.0,
            balance: 0.5,
            ..EqState::default()
        };
        state.bands.clear();
        state.graphic.gains_db[0] = 5.0;
        state.reset();
        assert_eq!(state.bands, default_bands());
        assert_eq!(state.graphic.gains_db[0], 5.0);
        assert_eq!((state.preamp_db, state.bass_db, state.balance), (0.0, 2.0, 0.5));
        assert!(!state.can_reset());

        state.mode = Mode::Graphic;
        assert!(state.can_reset());
        state.reset();
        assert!(state.graphic.gains_db.iter().all(|&g| g == 0.0));
        assert_eq!(state.graphic.band_count, 10);
    }

    #[test]
    fn new_bands_go_where_there_is_room() {
        let mut state = EqState::default();
        let mut selected = None;
        // The default bands sit at 1 kHz and 200 Hz, so 1 kHz and 250 Hz are taken.
        state.add_band(&mut selected);
        assert_eq!(state.bands.last().unwrap().freq, 4_000.0);
        assert_eq!(selected, Some(7));
        state.bands.clear();
        state.add_band(&mut selected);
        assert_eq!(state.bands[0].freq, 1_000.0);
    }

    #[test]
    fn changing_the_band_count_keeps_the_shape() {
        let mut graphic = Graphic {
            gains_db: vec![0.0, 2.0, 0.0, 4.0, 0.0, 6.0, 0.0, 8.0, 0.0, 10.0],
            ..Graphic::default()
        };
        graphic.set_band_count(5);
        assert_eq!(graphic.gains_db, [2.0, 4.0, 6.0, 8.0, 10.0]);
        graphic.set_band_count(7);
        assert_eq!(graphic.band_count, 5);
    }

    #[test]
    fn each_device_gets_its_own_curve_back() {
        let mut settings = with_preamp(-1.0);
        assert!(!settings.switch_device("speakers"));
        settings.save_preset("Speakers").unwrap();

        // A device seen for the first time starts from the current curve.
        assert!(!settings.switch_device("headphones"));
        assert_eq!(settings.eq.preamp_db, -1.0);
        settings.eq.preamp_db = -6.0;
        settings.eq.mode = Mode::Graphic;
        settings.eq.balance = 0.5;

        assert!(settings.switch_device("speakers"));
        assert_eq!(settings.eq.preamp_db, -1.0);
        assert_eq!((settings.eq.mode, settings.eq.balance), (Mode::Parametric, 0.0));
        assert_eq!(settings.active_preset.as_deref(), Some("Speakers"));
        assert!(settings.switch_device("headphones"));
        assert_eq!(settings.eq.preamp_db, -6.0);
        assert_eq!((settings.eq.mode, settings.eq.balance), (Mode::Graphic, 0.5));
        assert!(!settings.switch_device("headphones"));
    }

    #[test]
    fn values_that_are_not_numbers_become_safe_defaults() {
        let mut band = Band::new(FilterKind::Peak, f32::NAN);
        band.gain_db = f32::INFINITY;
        band.q = f32::NAN;
        let mut state = EqState {
            bands: vec![band],
            preamp_db: f32::NAN,
            bass_db: f32::NEG_INFINITY,
            treble_db: f32::NAN,
            balance: f32::NAN,
            ..EqState::default()
        };
        state.clamp();
        let band = state.bands[0];
        assert!(band.freq.is_finite() && band.q.is_finite());
        assert_eq!(band.gain_db, 0.0);
        assert_eq!((state.preamp_db, state.bass_db, state.treble_db, state.balance), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn a_not_a_number_never_reaches_the_file() {
        let mut settings = with_preamp(f32::NAN);
        settings.save_preset("Warm").unwrap();
        settings.eq.clamp();
        let settings = Settings::parse(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(settings.presets.len(), 1);
    }

    #[test]
    fn thousands_of_clashing_names_load_quickly() {
        let mut old = with_preamp(0.0);
        old.presets = (0..3_000)
            .map(|_| Preset {
                name: "Same".into(),
                curve: old.eq.curve(),
            })
            .collect();
        let bytes = serde_json::to_vec(&old).unwrap();
        let started = std::time::Instant::now();
        let settings = Settings::parse(&bytes).unwrap();
        let names: HashSet<_> = settings.presets.iter().map(|p| p.name.to_lowercase()).collect();
        assert_eq!(names.len(), 3_000);
        assert_eq!(settings.presets[2_999].name, "Same 3000");
        // This took over a minute when every name was compared with every other.
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }

    #[test]
    fn importing_never_replaces_a_preset_of_the_same_name() {
        let mut settings = with_preamp(-1.0);
        settings.save_preset("HD 650").unwrap();
        let curve = Curve {
            bands: vec![Band::new(FilterKind::Peak, 500.0)],
            preamp_db: -6.0,
            ..Curve::default()
        };
        assert_eq!(settings.import_preset("HD 650", &curve).as_deref(), Ok("HD 650 2"));
        assert_eq!(settings.import_preset("hd 650", &curve).as_deref(), Ok("hd 650 3"));
        assert_eq!(settings.presets[0].curve.preamp_db, -1.0);
        let long = "x".repeat(MAX_PRESET_NAME);
        settings.save_preset(&long).unwrap();
        let renamed = settings.import_preset(&long, &curve).unwrap();
        assert_eq!(renamed.chars().count(), MAX_PRESET_NAME);
        assert!(renamed.ends_with(" 2"));
    }

    #[test]
    fn an_unreadable_file_is_set_aside_instead_of_overwritten() {
        let dir = std::env::temp_dir().join(format!("eq-test-unreadable-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let newer = br#"{"bands":[{"kind":"Notch","freq":1000.0,"gain_db":0.0,"q":1.0,"enabled":true}],
            "preamp_db":0.0,"bypass":false,"presets":[{"name":"Mine","bands":[],"preamp_db":-2.0}]}"#;
        std::fs::write(&path, newer).unwrap();

        let (settings, problem) = Settings::load_from(&path);
        assert_eq!(settings, Settings::default());
        let kept = match problem {
            Some(LoadProblem::SetAside(kept)) => kept,
            other => panic!("{other:?}"),
        };
        assert_eq!(std::fs::read(&kept).unwrap(), newer);
        assert!(!path.exists());
        // A second unreadable file does not replace the first one set aside.
        std::fs::write(&path, b"{").unwrap();
        let (_, problem) = Settings::load_from(&path);
        assert!(matches!(problem, Some(LoadProblem::SetAside(ref other)) if *other != kept));
        assert_eq!(std::fs::read(&kept).unwrap(), newer);

        // No file at all is a first launch, not a problem.
        let (_, problem) = Settings::load_from(&dir.join("missing.json"));
        assert!(problem.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_writes_the_whole_file_and_loads_back() {
        let dir = std::env::temp_dir().join(format!("eq-test-save-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("state.json");
        let mut settings = with_preamp(-3.0);
        settings.save_preset("Warm").unwrap();
        settings.save_to(&path).unwrap();
        let (loaded, problem) = Settings::load_from(&path);
        assert!(problem.is_none());
        assert_eq!(loaded, settings);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_curve_everywhere_when_per_device_is_off() {
        let mut settings = with_preamp(-1.0);
        settings.per_device = false;
        settings.switch_device("speakers");
        settings.switch_device("headphones");
        settings.eq.preamp_db = -6.0;
        assert!(!settings.switch_device("speakers"));
        assert_eq!(settings.eq.preamp_db, -6.0);
    }
}
