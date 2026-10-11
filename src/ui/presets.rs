use super::theme;
use crate::state::Settings;
use eframe::egui::{Align, ComboBox, Context, Id, Key, Layout, Modal, RichText, TextEdit, Ui};

const PICKER_WIDTH: f32 = 150.0;

/// Something the user asked to do with presets, from the picker or the menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetAction {
    Select(usize),
    Save,
    SaveAs,
    Rename,
    Delete,
    Import,
}

/// The preset menu in the top bar. Its title is the active preset, marked once edited.
pub fn picker(ui: &mut Ui, settings: &Settings) -> Option<PresetAction> {
    let active = settings.active_index();
    let modified = settings.is_modified();
    let title = match active {
        Some(index) if modified => format!("{} · Edited", settings.presets[index].name),
        Some(index) => settings.presets[index].name.clone(),
        None => "Presets".into(),
    };

    let mut action = None;
    ComboBox::from_id_salt("preset")
        .selected_text(title)
        .width(PICKER_WIDTH)
        .truncate()
        .show_ui(ui, |ui| {
            for (index, preset) in settings.presets.iter().enumerate() {
                // Choosing the active preset again discards the edits made since.
                if ui.selectable_label(active == Some(index), &preset.name).clicked() {
                    action = Some(PresetAction::Select(index));
                }
            }
            if !settings.presets.is_empty() {
                ui.separator();
            }
            let mut item = |ui: &mut Ui, enabled: bool, text: &str, chosen: PresetAction| {
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.selectable_label(false, text).clicked() {
                        action = Some(chosen);
                    }
                });
            };
            item(ui, modified, "Save", PresetAction::Save);
            item(ui, true, "Save As…", PresetAction::SaveAs);
            item(ui, active.is_some(), "Rename…", PresetAction::Rename);
            item(ui, active.is_some(), "Delete…", PresetAction::Delete);
            ui.separator();
            item(ui, true, "Import…", PresetAction::Import);
        })
        .response
        .on_hover_text("Save this curve as a preset, or load one");
    action
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
}

impl Dialog {
    pub fn save_as(settings: &Settings) -> Dialog {
        Dialog {
            kind: Kind::SaveAs,
            name: settings.active_preset.clone().unwrap_or_default(),
            focused: false,
        }
    }

    pub fn rename(settings: &Settings) -> Option<Dialog> {
        let index = settings.active_index()?;
        Some(Dialog {
            kind: Kind::Rename(index),
            name: settings.presets[index].name.clone(),
            focused: false,
        })
    }

    pub fn delete(settings: &Settings) -> Option<Dialog> {
        Some(Dialog {
            kind: Kind::Delete(settings.active_index()?),
            name: String::new(),
            focused: false,
        })
    }

    /// Shows the sheet and applies it to `settings` when confirmed. Returns false once it
    /// has been confirmed or dismissed.
    pub fn show(&mut self, ctx: &Context, settings: &mut Settings) -> bool {
        let name = self.name.trim().to_owned();
        let exists = |skip: Option<usize>| {
            settings
                .presets
                .iter()
                .enumerate()
                .any(|(i, p)| Some(i) != skip && p.name == name)
        };
        let (prompt, confirm, valid) = match self.kind {
            Kind::SaveAs if exists(None) => ("Save preset as", "Replace", true),
            Kind::SaveAs => ("Save preset as", "Save", !name.is_empty()),
            Kind::Rename(index) => {
                ("Rename preset", "Rename", !name.is_empty() && !exists(Some(index)))
            }
            Kind::Delete(_) => ("", "Delete", true),
        };

        let mut confirmed = false;
        let mut cancelled = false;
        let modal = Modal::new(Id::new("preset-dialog")).show(ctx, |ui| {
            let p = &theme::palette(ui);
            ui.set_width(260.0);
            if let Kind::Delete(index) = self.kind {
                let name = settings.presets.get(index).map_or("", |p| p.name.as_str());
                ui.label(format!("Delete the preset “{name}”?"));
                ui.label(
                    RichText::new("The equalizer keeps its current curve. This cannot be undone.")
                        .small()
                        .color(p.secondary),
                );
            } else {
                ui.label(prompt);
                let field = ui.add(TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
                if !self.focused {
                    self.focused = true;
                    field.request_focus();
                }
                if matches!(self.kind, Kind::Rename(_)) && !valid && !name.is_empty() {
                    ui.label(
                        RichText::new("Another preset already has this name.")
                            .small()
                            .color(p.secondary),
                    );
                }
            }
            ui.add_space(4.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(valid, |ui| confirmed = ui.button(confirm).clicked());
                cancelled = ui.button("Cancel").clicked();
            });
        });
        confirmed |= valid && ctx.input(|i| i.key_pressed(Key::Enter));

        if confirmed {
            match self.kind {
                Kind::SaveAs => settings.save_preset(&name),
                Kind::Rename(index) => _ = settings.rename_preset(index, &name),
                Kind::Delete(index) => settings.delete_preset(index),
            }
        }
        !(confirmed || cancelled || modal.should_close())
    }
}
