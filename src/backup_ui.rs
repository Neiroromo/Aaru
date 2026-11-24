//! The window of the export and import of the personal data (see `backup`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use egui::{Context, Id, Modal, RichText};
use egui_file_dialog::{DialogState, FileDialog};

use crate::backup::{self, Choice, Inspected, Mode, Plan, PortableSettings, Roots, Section};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Export,
    Import,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Save,
    Open,
}

struct ImportState {
    path: PathBuf,
    found: Inspected,
    mode: Mode,
    sections: BTreeSet<Section>,
    /// The account of the file to the account here; 0 is "not imported".
    mapping: BTreeMap<u32, u32>,
}

/// What the window asks of the program.
pub enum Action {
    None,
    /// An import was put by: the settings it has to apply now (with where the accounts of the
    /// file go), and the program is to start again for the rest.
    Imported { settings: Option<(PortableSettings, BTreeMap<u32, u32>)> },
}

#[derive(Default)]
pub struct BackupUi {
    pub open: bool,
    tab: Option<Tab>,
    export_sections: BTreeSet<Section>,
    export_accounts: BTreeSet<u32>,
    local: BTreeMap<u32, String>,
    dialog: Option<(FileDialog, Kind)>,
    import: Option<ImportState>,
    /// What the last thing done came to: (it went well, text).
    message: Option<(bool, String)>,
}

impl BackupUi {
    pub fn open(&mut self) {
        self.open = true;
        self.message = None;
        self.import = None;
        self.tab = Some(Tab::Export);
        let accounts = Roots::current().map(|roots| roots.accounts()).unwrap_or_default();
        let names: BTreeMap<u32, String> = crate::profiles::profiles_dir()
            .map(|dir| crate::profiles::list_in(&dir))
            .unwrap_or_default()
            .into_iter()
            .map(|profile| (profile.uid, profile.nickname.unwrap_or_default()))
            .collect();
        self.local = accounts.iter().map(|uid| (*uid, names.get(uid).cloned().unwrap_or_default())).collect();
        self.export_accounts = accounts.into_iter().collect();
        self.export_sections = Section::ALL.into_iter().collect();
    }

    fn account_label(uid: u32, name: &str) -> String {
        if name.is_empty() { uid.to_string() } else { format!("{name} ({uid})") }
    }

    pub fn show(&mut self, ctx: &Context, russian: bool, current: &PortableSettings) -> Action {
        let mut action = Action::None;
        if !self.open {
            return action;
        }
        let t = |en: &'static str, ru: &'static str| if russian { ru } else { en };
        // The dialog to choose a file.
        if let Some((dialog, kind)) = &mut self.dialog {
            dialog.update(ctx);
            let picked = dialog.take_picked();
            let kind = *kind;
            let still_open = matches!(dialog.state(), DialogState::Open);
            if let Some(path) = picked {
                self.dialog = None;
                match kind {
                    Kind::Save => self.do_export(path, current, russian),
                    Kind::Open => self.do_inspect(path, russian),
                }
            } else if !still_open {
                self.dialog = None;
            }
        }
        if self.dialog.is_some() {
            return action;
        }
        let mut close = false;
        let modal = Modal::new(Id::new("Backup")).show(ctx, |ui| {
            ui.set_width(520.);
            ui.heading(t("Export and import of your data", "Экспорт и импорт ваших данных"));
            ui.add(
                egui::Label::new(
                    RichText::new(t(
                        "What the game cannot tell you again: what you marked on the map, chests and walked ground, planner goals, what was learned by playing, and some settings. Not the key of the connection, not caches or logs.",
                        "То, что игра повторно не расскажет: отметки на карте, сундуки и пройденные места, цели планировщика, выученное при игре и часть настроек. Без ключа соединения, кэшей и журналов.",
                    ))
                    .size(12.),
                )
                .wrap(),
            );
            ui.add_space(4.);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Some(Tab::Export), t("Export", "Экспорт"));
                ui.selectable_value(&mut self.tab, Some(Tab::Import), t("Import", "Импорт"));
            });
            ui.separator();
            match self.tab {
                Some(Tab::Import) => self.import_ui(ui, russian, &mut action),
                _ => self.export_ui(ui, russian),
            }
            if let Some((good, text)) = &self.message {
                ui.add_space(4.);
                let color = if *good { egui::Color32::from_rgb(110, 200, 120) } else { egui::Color32::from_rgb(230, 110, 110) };
                ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
            }
            ui.separator();
            if ui.button(t("Close", "Закрыть")).clicked() {
                close = true;
            }
        });
        if modal.should_close() || close {
            self.open = false;
        }
        action
    }

    fn export_ui(&mut self, ui: &mut egui::Ui, russian: bool) {
        let t = |en: &'static str, ru: &'static str| if russian { ru } else { en };
        ui.label(t("What to put in the file:", "Что положить в файл:"));
        for section in Section::ALL {
            let mut on = self.export_sections.contains(&section);
            if ui.checkbox(&mut on, section.title(russian)).on_hover_text(section.about(russian)).changed() {
                if on {
                    self.export_sections.insert(section);
                } else {
                    self.export_sections.remove(&section);
                }
            }
        }
        ui.add_space(4.);
        ui.label(t("Accounts:", "Аккаунты:"));
        if self.local.is_empty() {
            ui.label(RichText::new(t("nothing is kept for any account yet", "пока ничего не сохранено ни для одного аккаунта")).weak());
        }
        for (uid, name) in self.local.clone() {
            let mut on = self.export_accounts.contains(&uid);
            if ui.checkbox(&mut on, Self::account_label(uid, &name)).changed() {
                if on {
                    self.export_accounts.insert(uid);
                } else {
                    self.export_accounts.remove(&uid);
                }
            }
        }
        ui.add_space(6.);
        let any = !self.export_sections.is_empty();
        if ui.add_enabled(any, egui::Button::new(t("Save to a file...", "Сохранить в файл..."))).clicked() {
            let name = format!("aaru-backup-{}.zip", chrono::Local::now().format("%Y-%m-%d"));
            let mut dialog = FileDialog::new().add_file_filter_extensions("Backup", vec!["zip"]).default_file_name(&name);
            dialog.save_file();
            self.dialog = Some((dialog, Kind::Save));
        }
    }

    fn do_export(&mut self, mut path: PathBuf, current: &PortableSettings, russian: bool) {
        if path.extension().is_none() {
            path.set_extension("zip");
        }
        let choice = Choice { sections: self.export_sections.clone(), accounts: self.export_accounts.clone() };
        let result = Roots::current().and_then(|roots| {
            let mut settings = current.clone();
            // The overlay of the map is a file of its own.
            settings.overlay = std::fs::read(roots.data.join("map_overlay.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
            backup::export(&roots, &choice, Some(&settings), &path)
        });
        self.message = Some(match result {
            Ok(summary) => (
                true,
                if russian {
                    format!("Готово: {} файлов, {} КБ — {}", summary.files, summary.bytes / 1024, path.display())
                } else {
                    format!("Done: {} files, {} KB — {}", summary.files, summary.bytes / 1024, path.display())
                },
            ),
            Err(why) => (false, format!("{why:#}")),
        });
    }

    fn do_inspect(&mut self, path: PathBuf, russian: bool) {
        match backup::inspect(&path) {
            Ok(found) => {
                let mapping = found
                    .manifest
                    .accounts
                    .keys()
                    .map(|uid| (*uid, *uid))
                    .collect();
                self.import = Some(ImportState { path, mode: Mode::Merge, sections: found.manifest.sections.clone(), mapping, found });
                self.message = None;
            }
            Err(why) => {
                self.import = None;
                self.message = Some((false, if russian { format!("Не удалось прочитать файл: {why:#}") } else { format!("The file cannot be read: {why:#}") }));
            }
        }
    }

    fn import_ui(&mut self, ui: &mut egui::Ui, russian: bool, action: &mut Action) {
        let t = |en: &'static str, ru: &'static str| if russian { ru } else { en };
        if ui.button(t("Choose a backup file...", "Выбрать файл резервной копии...")).clicked() {
            let mut dialog = FileDialog::new().add_file_filter_extensions("Backup", vec!["zip"]);
            dialog.pick_file();
            self.dialog = Some((dialog, Kind::Open));
        }
        let local = self.local.clone();
        let Some(state) = &mut self.import else {
            return;
        };
        ui.add_space(4.);
        ui.label(format!(
            "{}  ·  Aaru {}  ·  {}",
            state.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            state.found.manifest.version,
            state.found.manifest.created.chars().take(10).collect::<String>()
        ));
        ui.add_space(4.);
        ui.label(t("What to take from it:", "Что взять из файла:"));
        for section in Section::ALL {
            if !state.found.manifest.sections.contains(&section) {
                continue;
            }
            let mut on = state.sections.contains(&section);
            if ui.checkbox(&mut on, section.title(russian)).on_hover_text(section.about(russian)).changed() {
                if on {
                    state.sections.insert(section);
                } else {
                    state.sections.remove(&section);
                }
            }
        }
        if !state.found.manifest.accounts.is_empty() {
            ui.add_space(4.);
            ui.label(t("Which account it goes to:", "В какой аккаунт попадает:"));
            for (uid, name) in state.found.manifest.accounts.clone() {
                ui.horizontal(|ui| {
                    ui.label(format!("{} →", Self::account_label(uid, &name)));
                    let chosen = state.mapping.entry(uid).or_insert(uid);
                    let shown = if *chosen == 0 {
                        t("not imported", "не импортировать").to_owned()
                    } else {
                        Self::account_label(*chosen, local.get(chosen).map(String::as_str).unwrap_or(""))
                    };
                    egui::ComboBox::from_id_salt(("backup-account", uid)).selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(chosen, uid, format!("{} ({})", uid, t("as it is", "как есть")));
                        for (other, other_name) in &local {
                            if *other != uid {
                                ui.selectable_value(chosen, *other, Self::account_label(*other, other_name));
                            }
                        }
                        ui.selectable_value(chosen, 0, t("not imported", "не импортировать"));
                    });
                });
            }
        }
        ui.add_space(4.);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut state.mode, Mode::Merge, t("Join", "Объединить"))
                .on_hover_text(t("What is here stays; what the file has more or later is added.", "То, что есть, остаётся; чего в файле больше или оно новее — добавляется."));
            ui.selectable_value(&mut state.mode, Mode::Replace, t("Replace", "Заменить"))
                .on_hover_text(t("The file takes the place of what is here (the old files are kept in import-backup).", "Файл заменяет то, что есть (старые файлы сохраняются в import-backup)."));
        });
        ui.add_space(4.);
        ui.add(
            egui::Label::new(
                RichText::new(t(
                    "The data is put in place when Aaru starts again; it starts again by itself after you press the button.",
                    "Данные встают на место при следующем запуске Aaru; после нажатия кнопки он перезапустится сам.",
                ))
                .size(12.),
            )
            .wrap(),
        );
        if ui.button(t("Import and restart", "Импортировать и перезапустить")).clicked() {
            let plan = Plan {
                mode: state.mode,
                sections: state.sections.clone(),
                accounts: state.mapping.iter().filter(|(_, to)| **to != 0).map(|(from, to)| (*from, *to)).collect(),
                owner_pid: 0,
            };
            let staged = Roots::current().and_then(|roots| backup::stage(&roots, &state.path, &plan));
            match staged {
                Ok(()) => {
                    let settings = state.sections.contains(&Section::Settings).then(|| state.found.settings.clone()).flatten().map(|s| (s, plan.accounts.clone()));
                    *action = Action::Imported { settings };
                }
                Err(why) => self.message = Some((false, format!("{why:#}"))),
            }
        }
    }
}
