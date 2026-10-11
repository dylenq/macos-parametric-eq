use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const MAX_BANDS: usize = 16;
pub const MIN_FREQ: f32 = 20.0;
pub const MAX_FREQ: f32 = 20_000.0;
pub const MIN_Q: f32 = 0.1;
pub const MAX_Q: f32 = 18.0;
pub const MAX_GAIN_DB: f32 = 24.0;

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
        self.freq = self.freq.clamp(MIN_FREQ, MAX_FREQ);
        self.q = self.q.clamp(MIN_Q, MAX_Q);
        self.gain_db = self.gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqState {
    pub bands: Vec<Band>,
    pub preamp_db: f32,
    pub bypass: bool,
}

impl Default for EqState {
    fn default() -> Self {
        let mut bands = vec![Band::new(FilterKind::LowShelf, 80.0)];
        for freq in [200.0, 500.0, 1_000.0, 2_500.0, 6_000.0] {
            let mut band = Band::new(FilterKind::Peak, freq);
            band.q = 1.0;
            bands.push(band);
        }
        bands.push(Band::new(FilterKind::HighShelf, 10_000.0));
        EqState {
            bands,
            preamp_db: 0.0,
            bypass: false,
        }
    }
}

impl EqState {
    /// Appends a flat 1 kHz peak band and selects it, if there is room.
    pub fn add_band(&mut self, selected: &mut Option<usize>) {
        if self.bands.len() < MAX_BANDS {
            let mut band = Band::new(FilterKind::Peak, 1_000.0);
            band.q = 1.0;
            self.bands.push(band);
            *selected = Some(self.bands.len() - 1);
        }
    }

    /// Restores the default flat bands and preamp. Bypass is left as it is.
    pub fn reset_bands(&mut self) {
        *self = EqState {
            bypass: self.bypass,
            ..EqState::default()
        };
    }

    fn curve(&self) -> Curve {
        Curve {
            bands: self.bands.clone(),
            preamp_db: self.preamp_db,
        }
    }

    fn set_curve(&mut self, curve: &Curve) {
        self.bands = curve.bands.clone();
        self.preamp_db = curve.preamp_db;
    }
}

/// The part of an [`EqState`] that presets and devices remember: everything but bypass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub bands: Vec<Band>,
    pub preamp_db: f32,
}

impl Curve {
    fn sanitize(&mut self) {
        self.bands.truncate(MAX_BANDS);
        self.bands.iter_mut().for_each(Band::clamp);
        self.preamp_db = self.preamp_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(flatten)]
    pub curve: Curve,
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
    pub fn active_index(&self) -> Option<usize> {
        let active = self.active_preset.as_deref()?;
        self.presets.iter().position(|p| p.name == active)
    }

    /// True if the equalizer has been edited since its preset was loaded or saved.
    pub fn is_modified(&self) -> bool {
        self.active_index()
            .is_some_and(|i| self.presets[i].curve != self.eq.curve())
    }

    pub fn apply_preset(&mut self, index: usize) {
        if let Some(preset) = self.presets.get(index) {
            self.eq.set_curve(&preset.curve);
            self.active_preset = Some(preset.name.clone());
        }
    }

    /// Saves the equalizer as it is now under `name`, replacing a preset of that name.
    pub fn save_preset(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let curve = self.eq.curve();
        match self.presets.iter_mut().find(|p| p.name == name) {
            Some(preset) => preset.curve = curve,
            None => self.presets.push(Preset {
                name: name.into(),
                curve,
            }),
        }
        self.active_preset = Some(name.into());
    }

    /// Loads a curve from elsewhere and keeps it as a preset.
    pub fn import_preset(&mut self, name: &str, curve: &Curve) {
        self.eq.set_curve(curve);
        self.save_preset(name);
    }

    /// Returns false if the name is empty or already used by another preset.
    pub fn rename_preset(&mut self, index: usize, name: &str) -> bool {
        let name = name.trim();
        let taken = self
            .presets
            .iter()
            .enumerate()
            .any(|(i, p)| i != index && p.name == name);
        if name.is_empty() || taken || index >= self.presets.len() {
            return false;
        }
        let old = std::mem::replace(&mut self.presets[index].name, name.into());
        self.for_each_reference(|active| {
            if active.as_deref() == Some(&old) {
                *active = Some(name.into());
            }
        });
        true
    }

    /// Removes a preset. The equalizer keeps sounding the way it does.
    pub fn delete_preset(&mut self, index: usize) {
        if index >= self.presets.len() {
            return;
        }
        let removed = self.presets.remove(index).name;
        self.for_each_reference(|active| {
            if active.as_deref() == Some(&removed) {
                *active = None;
            }
        });
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

    fn sanitize(&mut self) {
        let mut curve = self.eq.curve();
        curve.sanitize();
        self.eq.set_curve(&curve);
        self.presets.iter_mut().for_each(|p| p.curve.sanitize());
        self.devices.values_mut().for_each(|d| d.curve.sanitize());
    }

    fn path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("eq").join("state.json"))
    }

    fn parse(bytes: &[u8]) -> Self {
        let mut settings: Settings = serde_json::from_slice(bytes).unwrap_or_default();
        settings.sanitize();
        settings
    }

    pub fn load() -> Self {
        let bytes = Self::path().and_then(|p| std::fs::read(p).ok());
        Self::parse(bytes.as_deref().unwrap_or_default())
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_preamp(db: f32) -> Settings {
        let mut settings = Settings::default();
        settings.eq.preamp_db = db;
        settings
    }

    #[test]
    fn a_file_from_before_presets_still_loads() {
        let old = serde_json::to_vec(&EqState {
            preamp_db: -3.0,
            ..EqState::default()
        })
        .unwrap();
        let settings = Settings::parse(&old);
        assert_eq!(settings.eq.preamp_db, -3.0);
        assert!(settings.presets.is_empty());
        assert!(settings.per_device);
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let mut settings = with_preamp(-2.0);
        settings.save_preset("Warm");
        settings.switch_device("speakers");
        settings.switch_device("headphones");
        let bytes = serde_json::to_vec(&settings).unwrap();
        assert_eq!(Settings::parse(&bytes), settings);
    }

    #[test]
    fn unreadable_or_out_of_range_files_are_made_safe() {
        assert_eq!(Settings::parse(b"not json"), Settings::default());
        let mut wild = with_preamp(90.0);
        wild.save_preset("Loud");
        let settings = Settings::parse(&serde_json::to_vec(&wild).unwrap());
        assert_eq!(settings.eq.preamp_db, MAX_GAIN_DB);
        assert_eq!(settings.presets[0].curve.preamp_db, MAX_GAIN_DB);
    }

    #[test]
    fn presets_save_apply_and_track_edits() {
        let mut settings = with_preamp(-4.0);
        settings.save_preset("  Warm ");
        assert_eq!(settings.active_preset.as_deref(), Some("Warm"));
        assert!(!settings.is_modified());

        settings.eq.preamp_db = 1.0;
        assert!(settings.is_modified());
        settings.apply_preset(0);
        assert_eq!(settings.eq.preamp_db, -4.0);

        // Saving under the same name replaces the preset.
        settings.eq.preamp_db = 2.0;
        settings.save_preset("Warm");
        assert_eq!(settings.presets.len(), 1);
        assert_eq!(settings.presets[0].curve.preamp_db, 2.0);
    }

    #[test]
    fn applying_a_preset_leaves_bypass_alone() {
        let mut settings = with_preamp(-4.0);
        settings.save_preset("Warm");
        settings.eq.bypass = true;
        settings.apply_preset(0);
        assert!(settings.eq.bypass);
    }

    #[test]
    fn rename_and_delete_follow_the_active_preset() {
        let mut settings = with_preamp(0.0);
        settings.save_preset("A");
        settings.save_preset("B");
        assert!(!settings.rename_preset(1, "A"));
        assert!(!settings.rename_preset(1, " "));
        assert!(settings.rename_preset(1, "C"));
        assert_eq!(settings.active_preset.as_deref(), Some("C"));

        settings.delete_preset(1);
        assert_eq!(settings.active_preset, None);
        assert_eq!(settings.presets.len(), 1);
    }

    #[test]
    fn each_device_gets_its_own_curve_back() {
        let mut settings = with_preamp(-1.0);
        assert!(!settings.switch_device("speakers"));
        settings.save_preset("Speakers");

        // A device seen for the first time starts from the current curve.
        assert!(!settings.switch_device("headphones"));
        assert_eq!(settings.eq.preamp_db, -1.0);
        settings.eq.preamp_db = -6.0;

        assert!(settings.switch_device("speakers"));
        assert_eq!(settings.eq.preamp_db, -1.0);
        assert_eq!(settings.active_preset.as_deref(), Some("Speakers"));
        assert!(settings.switch_device("headphones"));
        assert_eq!(settings.eq.preamp_db, -6.0);
        assert!(!settings.switch_device("headphones"));
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
