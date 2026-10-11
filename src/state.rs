use serde::{Deserialize, Serialize};
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

    fn path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("eq").join("state.json"))
    }

    pub fn load() -> Self {
        let loaded = Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<EqState>(&bytes).ok());
        let mut state = loaded.unwrap_or_default();
        state.bands.truncate(MAX_BANDS);
        state.bands.iter_mut().for_each(Band::clamp);
        state.preamp_db = state.preamp_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
        state
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
