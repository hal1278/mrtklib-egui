//! Small in-window file/directory chooser; browsing never launches a process.

use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(crate) enum Target {
    Input,
    Directory,
    Output(usize),
}
struct Entry {
    path: PathBuf,
    directory: bool,
    label: String,
}
pub(crate) struct Picker {
    pub target: Target,
    directory: String,
    filename: String,
    entries: Vec<Entry>,
    error: Option<String>,
}
impl Picker {
    pub fn new(target: Target, value: &str) -> Self {
        let mut picker = Self {
            target,
            directory: String::new(),
            filename: String::new(),
            entries: Vec::new(),
            error: None,
        };
        match std::env::current_dir() {
            Ok(cwd) => {
                let path = match crate::naming::absolute(Path::new(value), &cwd) {
                    Ok(path) => path,
                    Err(error) => {
                        picker.error = Some(error);
                        return picker;
                    }
                };
                let directory = if value.is_empty() {
                    &cwd
                } else if matches!(target, Target::Directory) {
                    path.as_path()
                } else {
                    path.parent().unwrap_or(&cwd)
                };
                let Some(directory) = directory.to_str() else {
                    picker.error = Some("This directory cannot be represented as UTF-8".into());
                    return picker;
                };
                picker.directory = directory.to_owned();
                if !value.is_empty() && !matches!(target, Target::Directory) {
                    picker.filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                }
                picker.reload();
            }
            Err(error) => picker.error = Some(format!("cannot locate current directory: {error}")),
        }
        picker
    }
    fn reload(&mut self) {
        let result = std::fs::read_dir(&self.directory).and_then(|entries| {
            entries
                .map(|entry| {
                    let entry = entry?;
                    let directory = entry.metadata()?.is_dir();
                    let label = entry.file_name().to_string_lossy().into_owned();
                    Ok(Entry {
                        path: entry.path(),
                        directory,
                        label,
                    })
                })
                .collect::<Result<Vec<_>, std::io::Error>>()
        });
        match result {
            Ok(mut entries) => {
                entries.sort_by(|a, b| {
                    b.directory
                        .cmp(&a.directory)
                        .then_with(|| a.label.cmp(&b.label))
                });
                self.entries = entries;
                self.error = None;
            }
            Err(error) => {
                self.entries.clear();
                self.error = Some(format!("cannot browse {}: {error}", self.directory));
            }
        }
    }
    /// Return (selected path, closed). Cancel never changes the caller's value.
    pub fn show(&mut self, ctx: &egui::Context) -> (Option<PathBuf>, bool) {
        let mut selected = None;
        let mut closed = false;
        egui::Modal::new(egui::Id::new("file chooser")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(0.0, 620.0));
            ui.heading(match self.target {
                Target::Input => "Input file",
                Target::Directory => "Output Directory",
                Target::Output(_) => "Output file",
            });
            let mut reload = false;
            ui.horizontal(|ui| {
                if ui.button("Up").clicked()
                    && let Some(parent) = Path::new(&self.directory).parent()
                {
                    self.directory = parent.to_string_lossy().into_owned();
                    reload = true;
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.directory)
                        .desired_width(ui.available_width() - 45.0),
                );
                reload |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                reload |= ui.button("Go").clicked();
            });
            if reload {
                self.reload();
            }
            let mut navigate = None;
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 220.0).clamp(0.0, 320.0))
                .show(ui, |ui| {
                    for entry in &self.entries {
                        if matches!(self.target, Target::Directory) && !entry.directory {
                            continue;
                        }
                        let label = format!(
                            "{}{}",
                            if entry.directory { "[dir] " } else { "" },
                            mrtklib_egui_supervisor::mask::mask(&entry.label, &[])
                        );
                        let response = ui.selectable_label(
                            !entry.directory && self.filename == entry.label,
                            label,
                        );
                        if response.clicked() {
                            if entry.directory {
                                navigate = Some(entry.path.clone());
                            } else if entry.path.to_str().is_some() {
                                self.filename = entry.label.clone();
                            } else {
                                self.error =
                                    Some("This file name cannot be represented as UTF-8".into());
                            }
                        }
                    }
                });
            if let Some(directory) = navigate {
                if let Some(directory) = directory.to_str() {
                    self.directory = directory.to_owned();
                    self.reload();
                } else {
                    self.error = Some("This directory cannot be represented as UTF-8".into());
                }
            }
            if !matches!(self.target, Target::Directory) {
                ui.horizontal(|ui| {
                    ui.label("File name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filename)
                            .desired_width(ui.available_width()),
                    );
                });
            }
            if let Some(error) = &self.error {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    mrtklib_egui_supervisor::mask::mask(error, &[]),
                );
            }
            ui.horizontal(|ui| {
                if ui.button("Select").clicked() {
                    let path = match self.target {
                        Target::Directory => PathBuf::from(&self.directory),
                        _ => Path::new(&self.directory).join(&self.filename),
                    };
                    if path.to_str().is_none() {
                        self.error = Some("This path cannot be represented as UTF-8".into());
                    } else if !matches!(self.target, Target::Directory) && self.filename.is_empty()
                    {
                        self.error = Some("Enter a file name".into());
                    } else {
                        selected = Some(path);
                        closed = true;
                    }
                }
                closed |= ui.button("Cancel").clicked();
            });
        });
        (selected, closed)
    }
}
