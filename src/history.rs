use crate::state::{EqState, Settings};

const LIMIT: usize = 100;

/// What one undo step restores: the sound, and which preset it came from.
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    eq: EqState,
    active_preset: Option<String>,
}

impl Snapshot {
    fn of(settings: &Settings) -> Snapshot {
        Snapshot {
            eq: settings.eq.clone(),
            active_preset: settings.active_preset.clone(),
        }
    }

    /// Bypass and clip protection are ways of listening, not edits, so they stay as they
    /// are. The mode comes back with the rest, so an undo always shows what it changed.
    fn restore(self, settings: &mut Settings) {
        let (bypass, clip_protection) = (settings.eq.bypass, settings.eq.clip_protection);
        settings.eq = self.eq;
        settings.eq.bypass = bypass;
        settings.eq.clip_protection = clip_protection;
        // The preset may have been deleted since.
        settings.active_preset = self.active_preset;
        if settings.active_index().is_none() {
            settings.active_preset = None;
        }
    }
}

/// Undo history for edits to the sound. Switching mode on its own, bypass and clip
/// protection are never recorded.
#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The state from before the edit in progress, such as a drag that is still going.
    pending: Option<Snapshot>,
    /// Counts changes to the history, so a message can tell whether its Undo still applies.
    serial: u64,
}

fn edited(a: &EqState, b: &EqState) -> bool {
    a.bands != b.bands
        || a.graphic != b.graphic
        || a.preamp_db != b.preamp_db
        || a.bass_db != b.bass_db
        || a.treble_db != b.treble_db
        || a.balance != b.balance
}

impl History {
    /// Call once per frame with the settings before and after that frame's edits. A run of
    /// edited frames, or one held pointer gesture, becomes a single undo step.
    pub fn observe(&mut self, before: &Settings, after: &Settings, gesture_active: bool) {
        if edited(&before.eq, &after.eq) {
            self.pending.get_or_insert_with(|| Snapshot::of(before));
        } else if !gesture_active {
            self.commit();
        }
    }

    /// Ends the step in progress, so the next edit, such as applying a preset, is a step
    /// of its own.
    pub fn commit(&mut self) {
        if let Some(state) = self.pending.take() {
            self.undo.push(state);
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.serial += 1;
        }
    }

    /// Forgets every step. The serial keeps counting, so no message's Undo can match a
    /// later step by accident.
    pub fn clear(&mut self) {
        *self = History {
            serial: self.serial + 1,
            ..History::default()
        };
    }

    /// Changes whenever a step is added, undone or redone.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn can_undo(&self) -> bool {
        self.pending.is_some() || !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        self.pending.is_none() && !self.redo.is_empty()
    }

    /// Returns true if `settings` changed.
    pub fn undo(&mut self, settings: &mut Settings) -> bool {
        self.commit();
        let Some(saved) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot::of(settings));
        saved.restore(settings);
        self.serial += 1;
        true
    }

    /// Returns true if `settings` changed.
    pub fn redo(&mut self, settings: &mut Settings) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(saved) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot::of(settings));
        saved.restore(settings);
        self.serial += 1;
        true
    }

    /// Follows a preset that was renamed, or forgets one that was deleted (`new` is None).
    pub fn rekey(&mut self, old: &str, new: Option<&str>) {
        let snapshots = self.undo.iter_mut().chain(&mut self.redo).chain(&mut self.pending);
        for snapshot in snapshots {
            if snapshot.active_preset.as_deref() == Some(old) {
                snapshot.active_preset = new.map(str::to_owned);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Mode, builtins};

    fn with_preamp(db: f32) -> Settings {
        let mut settings = Settings::default();
        settings.eq.preamp_db = db;
        settings
    }

    #[test]
    fn a_drag_is_one_step_and_redo_restores_it() {
        let mut history = History::default();
        // Three frames of one drag, then the pointer is released.
        history.observe(&with_preamp(0.0), &with_preamp(1.0), true);
        history.observe(&with_preamp(1.0), &with_preamp(2.0), true);
        history.observe(&with_preamp(2.0), &with_preamp(2.0), true);
        history.observe(&with_preamp(2.0), &with_preamp(2.0), false);

        let mut settings = with_preamp(2.0);
        assert!(history.undo(&mut settings));
        assert_eq!(settings.eq.preamp_db, 0.0);
        assert!(!history.undo(&mut settings));
        assert!(history.redo(&mut settings));
        assert_eq!(settings.eq.preamp_db, 2.0);
        assert!(!history.can_redo());
    }

    #[test]
    fn bypass_and_mode_switches_are_not_recorded() {
        let mut history = History::default();
        let mut bypassed = with_preamp(0.0);
        bypassed.eq.bypass = true;
        history.observe(&with_preamp(0.0), &bypassed, false);
        let mut graphic = bypassed.clone();
        graphic.eq.mode = Mode::Graphic;
        history.observe(&bypassed, &graphic, false);
        assert!(!history.can_undo());

        let mut edited = graphic.clone();
        edited.eq.preamp_db = 3.0;
        history.observe(&graphic, &edited, false);
        history.observe(&edited, &edited, false);
        assert!(history.undo(&mut edited));
        assert_eq!(edited.eq.preamp_db, 0.0);
        assert!(edited.eq.bypass);
    }

    #[test]
    fn undo_brings_back_the_mode_the_edit_was_made_in() {
        let mut history = History::default();
        let mut settings = Settings::default();
        settings.eq.mode = Mode::Graphic;
        let before = settings.clone();
        settings.eq.graphic.gains_db[0] = 5.0;
        history.observe(&before, &settings, false);
        history.observe(&settings, &settings, false);
        settings.eq.mode = Mode::Parametric;

        assert!(history.undo(&mut settings));
        assert_eq!(settings.eq.mode, Mode::Graphic);
        assert_eq!(settings.eq.graphic.gains_db[0], 0.0);
    }

    #[test]
    fn applying_a_preset_is_one_step_with_its_marker() {
        let mut history = History::default();
        let mut settings = with_preamp(-1.0);
        settings.save_preset("Mine").unwrap();
        let before = settings.clone();
        settings.apply_preset(1);
        history.commit();
        history.observe(&before, &settings, false);
        history.observe(&settings, &settings, false);
        let serial = history.serial();

        assert!(history.undo(&mut settings));
        assert_eq!(settings.active_preset.as_deref(), Some("Mine"));
        assert_eq!(settings.eq.mode, Mode::Parametric);
        assert_ne!(history.serial(), serial);
        assert!(history.redo(&mut settings));
        assert_eq!(settings.active_preset.as_deref(), Some(builtins()[1].name.as_str()));
        assert_eq!(settings.eq.mode, Mode::Graphic);
    }

    #[test]
    fn renamed_and_deleted_presets_are_followed() {
        let mut history = History::default();
        let mut settings = with_preamp(-1.0);
        settings.save_preset("Old").unwrap();
        let before = settings.clone();
        settings.eq.preamp_db = -2.0;
        history.observe(&before, &settings, false);
        history.observe(&settings, &settings, false);

        let index = settings.active_index().unwrap();
        settings.rename_preset(index, "New").unwrap();
        history.rekey("Old", Some("New"));
        history.undo(&mut settings);
        assert_eq!(settings.active_preset.as_deref(), Some("New"));

        history.redo(&mut settings);
        settings.delete_preset(index).unwrap();
        history.rekey("New", None);
        history.undo(&mut settings);
        assert_eq!(settings.active_preset, None);
    }
}
