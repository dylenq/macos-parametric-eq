use super::theme;
use crate::state::{MAX_PRESET_NAME, PresetError, Settings, builtins, check_name};
use eframe::egui::{Align, ComboBox, Context, Id, Key, Layout, Modal, RichText, TextEdit, Ui};

const PICKER_WIDTH: f32 = 140.0;

/// Something the user asked to do with presets, from the picker or the menu bar. Indexes
/// count the built-in presets first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetAction {
    Select(usize),
    Save,
    SaveAs,
    /// Rename or delete the active preset.
    Rename,
    Delete,
    /// Rename or delete a preset from its own row.
    RenameAt(usize),
    DeleteAt(usize),
    Import,
}

/// The active preset, if it is one of the user's own and so can be changed.
pub fn active_user_preset(settings: &Settings) -> Option<usize> {
    settings.active_index().filter(|&i| !Settings::is_builtin(i))
}

/// The preset menu in the top bar. Its title is the active preset, marked once edited.
pub fn picker(ui: &mut Ui, settings: &Settings) -> Option<PresetAction> {
    let p = &theme::palette(ui);
    let active = settings.active_index();
    let modified = settings.is_modified();
    let title = match active.and_then(|i| settings.preset(i)) {
        Some(preset) if modified => format!("{} · Edited", preset.name),
        Some(preset) => preset.name.clone(),
        None => "Presets".into(),
    };
    let own = active_user_preset(settings);

    let mut action = None;
    ComboBox::from_id_salt("preset")
        .selected_text(title)
        .width(PICKER_WIDTH)
        .truncate()
        .show_ui(ui, |ui| {
            let heading = |ui: &mut Ui, text: &str| {
                ui.label(RichText::new(text).small().color(p.secondary));
            };
            heading(ui, "Built-in");
            for (index, preset) in builtins().iter().enumerate() {
                // Choosing the active preset again discards the edits made since.
                if ui.selectable_label(active == Some(index), &preset.name).clicked() {
                    action = Some(PresetAction::Select(index));
                }
            }
            ui.separator();
            heading(ui, "Your Presets");
            if settings.presets.is_empty() {
                ui.label(RichText::new("Save the current sound to reuse it").color(p.secondary));
            }
            for (n, preset) in settings.presets.iter().enumerate() {
                let index = builtins().len() + n;
                let row = ui
                    .selectable_label(active == Some(index), &preset.name)
                    .on_hover_text("Right-click to rename or delete");
                if row.clicked() {
                    action = Some(PresetAction::Select(index));
                }
                row.context_menu(|ui| {
                    if ui.button("Rename…").clicked() {
                        action = Some(PresetAction::RenameAt(index));
                        ui.close();
                    }
                    if ui.button("Delete…").clicked() {
                        action = Some(PresetAction::DeleteAt(index));
                        ui.close();
                    }
                });
            }
            ui.separator();
            let mut item = |ui: &mut Ui, enabled: bool, text: &str, chosen: PresetAction| {
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.selectable_label(false, text).clicked() {
                        action = Some(chosen);
                    }
                });
            };
            item(ui, modified && own.is_some(), "Save", PresetAction::Save);
            item(ui, true, "Save As…", PresetAction::SaveAs);
            item(ui, own.is_some(), "Rename…", PresetAction::Rename);
            item(ui, own.is_some(), "Delete…", PresetAction::Delete);
            ui.separator();
            item(ui, true, "Import…", PresetAction::Import);
        })
        .response
        .on_hover_text("Save this sound as a preset, or load one");
    action
}

/// What a confirmed sheet did, for the status message and the undo history.
pub enum Outcome {
    Saved(String),
    Renamed { old: String, new: String },
    Deleted(String),
}

enum Kind {
    SaveAs,
    Rename(usize),
    Delete(usize),
}

/// A sheet asking for a preset name, or to confirm deleting one.
pub struct Dialog {
    kind: Kind,
    name: String,
    /// The name field takes the keyboard once, when the sheet opens.
    focused: bool,
    /// Why the last attempt failed, when the check could not be made in advance.
    error: Option<PresetError>,
    /// The sheet has been shown before. The Return that opened it must not also confirm it.
    shown: bool,
}

impl Dialog {
    fn new(kind: Kind, name: String) -> Dialog {
        Dialog {
            kind,
            name,
            focused: false,
            error: None,
            shown: false,
        }
    }

    pub fn save_as(settings: &Settings) -> Dialog {
        let name = active_user_preset(settings)
            .and_then(|i| settings.preset(i))
            .map(|p| p.name.clone())
            .unwrap_or_default();
        Dialog::new(Kind::SaveAs, name)
    }

    pub fn rename(settings: &Settings, index: usize) -> Option<Dialog> {
        let preset = settings.preset(index).filter(|_| !Settings::is_builtin(index))?;
        Some(Dialog::new(Kind::Rename(index), preset.name.clone()))
    }

    pub fn delete(settings: &Settings, index: usize) -> Option<Dialog> {
        settings.preset(index).filter(|_| !Settings::is_builtin(index))?;
        Some(Dialog::new(Kind::Delete(index), String::new()))
    }

    /// Whether `name` would be accepted, and whether it replaces a preset of that name.
    fn check(&self, settings: &Settings) -> (Result<(), PresetError>, bool) {
        let (Kind::SaveAs | Kind::Rename(_)) = self.kind else {
            return (Ok(()), false);
        };
        let name = match check_name(&self.name) {
            Ok(name) => name.to_lowercase(),
            Err(error) => return (Err(error), false),
        };
        let skip = match self.kind {
            Kind::Rename(index) => Some(index),
            _ => None,
        };
        let clash = settings
            .all_presets()
            .enumerate()
            .find(|(i, p)| Some(*i) != skip && p.name.trim().to_lowercase() == name);
        match (clash, &self.kind) {
            (Some((i, _)), Kind::SaveAs) if !Settings::is_builtin(i) => (Ok(()), true),
            (Some(_), _) => (Err(PresetError::Taken), false),
            (None, Kind::SaveAs) if settings.presets.len() >= crate::state::MAX_USER_PRESETS => {
                (Err(PresetError::TooMany), false)
            }
            (None, _) => (Ok(()), false),
        }
    }

    /// Shows the sheet and applies it to `settings` when confirmed. Returns None while it
    /// is open, and Some once it has been confirmed or dismissed, with what it did.
    pub fn show(&mut self, ctx: &Context, settings: &mut Settings) -> Option<Option<Outcome>> {
        let (checked, replaces) = self.check(settings);
        let error = checked.err().or(self.error);
        let valid = error.is_none();
        let (prompt, confirm) = match self.kind {
            Kind::SaveAs if replaces => ("Save preset as", "Replace"),
            Kind::SaveAs => ("Save preset as", "Save"),
            Kind::Rename(_) => ("Rename preset", "Rename"),
            Kind::Delete(_) => ("", "Delete"),
        };

        let mut confirmed = false;
        let mut cancelled = false;
        let modal = Modal::new(Id::new("preset-dialog")).show(ctx, |ui| {
            let p = &theme::palette(ui);
            ui.set_width(260.0);
            if let Kind::Delete(index) = self.kind {
                let name = settings.preset(index).map_or("", |p| p.name.as_str());
                ui.label(format!("Delete the preset “{name}”?"));
                ui.label(
                    RichText::new("The equalizer keeps its current sound. This cannot be undone.")
                        .small()
                        .color(p.secondary),
                );
            } else {
                ui.label(prompt);
                let field = ui.add(
                    TextEdit::singleline(&mut self.name)
                        .char_limit(MAX_PRESET_NAME)
                        .desired_width(f32::INFINITY),
                );
                if field.changed() {
                    self.error = None;
                }
                if !self.focused {
                    self.focused = true;
                    field.request_focus();
                }
                let help = match error {
                    // An empty field says what it needs once something has been typed.
                    Some(PresetError::Blank) => "Up to 40 characters".to_owned(),
                    Some(error) => error.to_string(),
                    None if replaces => "Replaces the preset with this name.".to_owned(),
                    None => "Up to 40 characters".to_owned(),
                };
                ui.label(RichText::new(help).small().color(p.secondary));
            }
            ui.add_space(4.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(valid, |ui| confirmed = ui.button(confirm).clicked());
                cancelled = ui.button("Cancel").clicked();
            });
        });
        confirmed |= valid && self.shown && ctx.input(|i| i.key_pressed(Key::Enter));
        self.shown = true;

        if confirmed {
            let result = match self.kind {
                Kind::SaveAs => settings.save_preset(&self.name).map(|()| {
                    Outcome::Saved(settings.active_preset.clone().unwrap_or_default())
                }),
                Kind::Rename(index) => {
                    let old = settings.preset(index).map(|p| p.name.clone()).unwrap_or_default();
                    settings
                        .rename_preset(index, &self.name)
                        .map(|new| Outcome::Renamed { old, new })
                }
                Kind::Delete(index) => settings.delete_preset(index).map(Outcome::Deleted),
            };
            return match result {
                Ok(outcome) => Some(Some(outcome)),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            };
        }
        (cancelled || modal.should_close()).then_some(None)
    }
}
