use crate::state::EqState;

const LIMIT: usize = 100;

/// Undo history for band and preamp edits. Bypass and clip protection are ways of
/// listening, not edits, so they are never recorded or restored.
#[derive(Default)]
pub struct History {
    undo: Vec<EqState>,
    redo: Vec<EqState>,
    /// The state from before the edit in progress, such as a drag that is still going.
    pending: Option<EqState>,
}

fn edited(a: &EqState, b: &EqState) -> bool {
    a.bands != b.bands || a.preamp_db != b.preamp_db
}

fn restore(state: &mut EqState, saved: EqState) {
    let (bypass, clip_protection) = (state.bypass, state.clip_protection);
    *state = saved;
    state.bypass = bypass;
    state.clip_protection = clip_protection;
}

impl History {
    /// Call once per frame with the state before and after that frame's edits. A run of
    /// edited frames, or one held pointer gesture, becomes a single undo step.
    pub fn observe(&mut self, before: &EqState, after: &EqState, gesture_active: bool) {
        if edited(before, after) {
            self.pending.get_or_insert_with(|| before.clone());
        } else if !gesture_active {
            self.commit();
        }
    }

    fn commit(&mut self) {
        if let Some(state) = self.pending.take() {
            self.undo.push(state);
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        self.pending.is_some() || !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        self.pending.is_none() && !self.redo.is_empty()
    }

    /// Returns true if `state` changed.
    pub fn undo(&mut self, state: &mut EqState) -> bool {
        self.commit();
        let Some(saved) = self.undo.pop() else {
            return false;
        };
        self.redo.push(state.clone());
        restore(state, saved);
        true
    }

    /// Returns true if `state` changed.
    pub fn redo(&mut self, state: &mut EqState) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(saved) = self.redo.pop() else {
            return false;
        };
        self.undo.push(state.clone());
        restore(state, saved);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_preamp(db: f32) -> EqState {
        EqState {
            preamp_db: db,
            ..EqState::default()
        }
    }

    #[test]
    fn a_drag_is_one_step_and_redo_restores_it() {
        let mut history = History::default();
        // Three frames of one drag, then the pointer is released.
        history.observe(&with_preamp(0.0), &with_preamp(1.0), true);
        history.observe(&with_preamp(1.0), &with_preamp(2.0), true);
        history.observe(&with_preamp(2.0), &with_preamp(2.0), true);
        history.observe(&with_preamp(2.0), &with_preamp(2.0), false);

        let mut state = with_preamp(2.0);
        assert!(history.undo(&mut state));
        assert_eq!(state.preamp_db, 0.0);
        assert!(!history.undo(&mut state));
        assert!(history.redo(&mut state));
        assert_eq!(state.preamp_db, 2.0);
        assert!(!history.can_redo());
    }

    #[test]
    fn bypass_is_not_recorded_or_restored() {
        let mut history = History::default();
        let mut bypassed = with_preamp(0.0);
        bypassed.bypass = true;
        history.observe(&with_preamp(0.0), &bypassed, false);
        assert!(!history.can_undo());

        history.observe(&bypassed, &with_preamp(3.0), false);
        let mut state = with_preamp(3.0);
        state.bypass = true;
        assert!(history.undo(&mut state));
        assert_eq!(state.preamp_db, 0.0);
        assert!(state.bypass);
    }
}
