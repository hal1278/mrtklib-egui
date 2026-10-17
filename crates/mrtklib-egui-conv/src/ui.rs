//! Draw controls and forward actions. Process ownership stays in Controller.

use crate::{
    controller::Controller,
    picker::{Picker, Target},
    settings::{ConvertOptions, FORMATS, OUTPUTS, SYSTEMS, TimeField, VERSIONS},
};
use eframe::egui;

const INTERVALS: [&str; 12] = [
    "0", "0.05", "0.1", "0.2", "0.25", "0.5", "1", "5", "10", "15", "30", "60",
];
const FREQUENCY_HELP: &str = "-f N takes the first N MRTKLIB frequency slots per system:\nGPS L1 L2 L5; GLONASS G1 G2; Galileo E1 E5a E5b E6 E5ab; QZSS L1 L5 L2 L6; SBAS L1 L5; BeiDou B1I B3 B2I/B2b B1C B2a; NavIC L5 S";

pub(crate) struct Conv {
    controller: Controller,
    options: Option<ConvertOptions>,
    options_error: Option<String>,
    picker: Option<Picker>,
    close_focused: bool,
}
impl Conv {
    pub fn new(ctx: &egui::Context) -> Self {
        let ctx = ctx.clone();
        Self {
            controller: Controller::new(move || ctx.request_repaint()),
            options: None,
            options_error: None,
            picker: None,
            close_focused: false,
        }
    }

    fn main_controls(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.controller.settings;
        let before_input = s.input.clone();
        let before_naming = (s.directory_enabled, s.directory.clone());
        let mut browse = None;
        ui.horizontal(|ui| {
            time(ui, "Time Start (GPST)", &mut s.start);
            time(ui, "Time End (GPST)", &mut s.end);
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.interval_enabled, "Interval");
            ui.add_enabled_ui(s.interval_enabled, |ui| {
                ui.add(egui::TextEdit::singleline(&mut s.interval).desired_width(58.0));
                egui::ComboBox::from_id_salt("interval choices")
                    .selected_text("s")
                    .width(42.0)
                    .show_ui(ui, |ui| {
                        for choice in INTERVALS {
                            ui.selectable_value(&mut s.interval, choice.into(), choice);
                        }
                    });
            });
            ui.add_enabled_ui(false, |ui| {
                ui.checkbox(&mut false, "Unit");
                ui.add(egui::TextEdit::singleline(&mut "24".to_owned()).desired_width(40.0));
                ui.label("H");
            })
            .response
            .on_disabled_hover_text("mrtk convert cannot split sessions into units");
        });
        ui.separator();
        ui.label("RTCM, RCV RAW or RINEX OBS");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut s.input)
                    .desired_width((ui.available_width() - 220.0).max(50.0)),
            );
            if ui.button("...").clicked() {
                browse = Some((Target::Input, s.input.clone()));
            }
            ui.label("Format");
            egui::ComboBox::from_id_salt("input format")
                .selected_text(FORMATS[s.format].0)
                .show_ui(ui, |ui| {
                    for (index, (label, _)) in FORMATS.iter().enumerate() {
                        ui.selectable_value(&mut s.format, index, *label);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.directory_enabled, "Output Directory");
            ui.add_enabled_ui(s.directory_enabled, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut s.directory)
                        .desired_width(ui.available_width() - 36.0),
                );
                if ui.button("...").clicked() {
                    browse = Some((Target::Directory, s.directory.clone()));
                }
            });
        });
        if before_input != s.input {
            self.controller.refresh_outputs(true);
        } else if before_naming != (s.directory_enabled, s.directory.clone()) {
            self.controller.refresh_outputs(false);
        }
        let s = &mut self.controller.settings;
        ui.label("RINEX OBS/NAV/GNAV/HNAV/QNAV/LNAV/CNAV/INAV and SBS");
        egui::Grid::new("outputs").num_columns(3).show(ui, |ui| {
            for (row, (label, _, _)) in OUTPUTS.iter().enumerate() {
                let available = s.row_available(row);
                ui.add_enabled(
                    available,
                    egui::Checkbox::new(&mut s.outputs[row].enabled, *label),
                )
                .on_disabled_hover_text(if row == 8 {
                    "SBS is unavailable for RINEX input"
                } else {
                    "Separate navigation outputs require RINEX 2.x; Sep NAV is unavailable"
                });
                let enabled = s.output_enabled(row);
                if ui
                    .add_enabled(
                        enabled,
                        egui::TextEdit::singleline(&mut s.outputs[row].path)
                            .desired_width((ui.available_width() - 44.0).max(50.0)),
                    )
                    .changed()
                {
                    s.outputs[row].edited = true;
                }
                if ui.add_enabled(enabled, egui::Button::new("...")).clicked() {
                    browse = Some((Target::Output(row), s.outputs[row].path.clone()));
                }
                ui.end_row();
            }
        });
        if let Some((target, path)) = browse {
            self.picker = Some(Picker::new(target, &path));
        }
    }

    fn options_dialog(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.options else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("conversion options")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(0.0, 660.0));
            ui.heading("Options");
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 160.0).max(0.0)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("RINEX Ver");
                    egui::ComboBox::from_id_salt("rinex version").selected_text(&draft.version).show_ui(ui, |ui| {
                        for version in VERSIONS { ui.selectable_value(&mut draft.version, version.into(), version); }
                    });
                    disabled_check(ui, "Sep NAV", false, "mrtk convert cannot separate RINEX 3 navigation");
                    ui.label("Station ID");
                    ui.add_enabled(draft.rinex2_name, egui::TextEdit::singleline(&mut draft.station).desired_width(70.0)).on_disabled_hover_text("Station ID is passed only with RINEX2 Name");
                    ui.checkbox(&mut draft.rinex2_name, "RINEX2 Name");
                });
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("RunBy/Obsrv/Agency");
                        ui.add_enabled(false, egui::TextEdit::singleline(&mut String::new()).desired_width(100.0)).on_disabled_hover_text("mrtk convert has no RunBy argument");
                        edits(ui, &mut draft.observer);
                    });
                    fields(ui, "Comment 1", std::slice::from_mut(&mut draft.comments[0]));
                    fields(ui, "Comment 2", std::slice::from_mut(&mut draft.comments[1]));
                    fields(ui, "Marker Name/#/Type", &mut draft.marker);
                    fields(ui, "Rec #/Type/Vers", &mut draft.receiver);
                    fields(ui, "Ant #/Type", &mut draft.antenna);
                    ui.horizontal(|ui| {
                        disabled_check(ui, "Approx Pos XYZ", false, "Fixed position is unavailable: decoded station position may replace XYZ");
                        edits(ui, &mut draft.position);
                    });
                    fields(ui, "Antenna Delta H/E/N", &mut draft.delta);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut draft.iono, "Iono Corr"); ui.checkbox(&mut draft.time_corr, "Time Corr"); ui.checkbox(&mut draft.leap_sec, "Leap Sec");
                        disabled_check(ui, "Phase Shift", false, "mrtk convert has no phase shift argument"); ui.checkbox(&mut draft.half_cycle, "Half Cyc Corr");
                    });
                    ui.label("Slash-joined fields cannot contain '/' or empty fields before nonempty fields.");
                });
                ui.horizontal(|ui| {
                    ui.group(|ui| {
                        ui.label("Observation Types");
                        ui.horizontal(|ui| { for label in ["C", "L", "D", "S"] { disabled_check(ui, label, true, "All four observation types are always on in mrtk convert"); } });
                    });
                    ui.group(|ui| {
                        ui.label("Frequencies");
                        egui::ComboBox::from_id_salt("frequencies").selected_text(draft.frequencies.to_string()).show_ui(ui, |ui| {
                            for count in 1..=5 { ui.selectable_value(&mut draft.frequencies, count, count.to_string()); }
                        }).response.on_hover_text(FREQUENCY_HELP);
                        ui.add_enabled(false, egui::Button::new("Mask...")).on_disabled_hover_text("later slice");
                    });
                });
                ui.group(|ui| {
                    ui.label("Satellite Systems");
                    ui.horizontal_wrapped(|ui| {
                        for (checked, (label, _)) in draft.systems.iter_mut().zip(SYSTEMS) { ui.checkbox(checked, label); }
                    });
                });
                fields(ui, "Excluded Satellites", std::slice::from_mut(&mut draft.excluded));
                fields(ui, "Receiver Options", std::slice::from_mut(&mut draft.receiver_options));
                ui.horizontal(|ui| {
                    ui.label("Time Tolerance (s)"); ui.add(egui::TextEdit::singleline(&mut draft.tolerance).desired_width(75.0));
                    ui.add_enabled(false, egui::Button::new("Debug: OFF")).on_disabled_hover_text("mrtk convert writes no trace file");
                    ui.add_enabled(false, egui::Button::new("FCN...")).on_disabled_hover_text("mrtk convert has no FCN argument");
                });
            });
            if let Some(error) = &self.options_error { ui.colored_label(egui::Color32::DARK_RED, mrtklib_egui_supervisor::mask::mask(error, &[])); }
            ui.horizontal(|ui| { ok = ui.button("OK").clicked(); cancel = ui.button("Cancel").clicked(); });
        });
        if ok {
            match draft.validate() {
                Ok(()) => {
                    self.controller.settings.options = draft.clone();
                    self.controller.refresh_outputs(false);
                    self.options = None;
                    self.options_error = None;
                }
                Err(error) => self.options_error = Some(error),
            }
        } else if cancel {
            self.options = None;
            self.options_error = None;
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // Close confirmation takes precedence; a native close request can arrive
        // while a different dialog is open.
        if self.controller.close_dialog {
            let mut wait = false;
            let mut kill = false;
            egui::Modal::new(egui::Id::new("close conversion")).show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(0.0, 500.0));
                ui.heading("Conversion is running");
                ui.label("Killing the conversion may leave output files incomplete.");
                ui.horizontal(|ui| {
                    let button = ui.button("Wait");
                    if !self.close_focused {
                        button.request_focus();
                        self.close_focused = true;
                    }
                    wait = button.clicked();
                    kill = ui.button("Kill and close").clicked();
                });
            });
            if wait {
                self.controller.close_dialog = false;
                self.close_focused = false;
            }
            if kill {
                self.controller.kill_and_close();
            }
            return;
        }
        self.close_focused = false;
        if let Some(pending) = &self.controller.overwrite {
            let mut overwrite = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("overwrite conversion outputs")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(0.0, 580.0));
                ui.heading("Existing output files");
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 160.0).max(0.0))
                    .show(ui, |ui| {
                        for path in &pending.paths {
                            ui.label(mrtklib_egui_supervisor::mask::mask(
                                &path.to_string_lossy(),
                                &[],
                            ));
                        }
                    });
                ui.horizontal(|ui| {
                    overwrite = ui.button("Overwrite").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if overwrite {
                self.controller.confirm_overwrite();
            } else if cancel {
                self.controller.overwrite = None;
            }
            return;
        }
        if let Some(picker) = &mut self.picker {
            let (selected, closed) = picker.show(ctx);
            if let Some(path) = selected {
                let value = path.to_str().expect("picker checks UTF-8").to_owned();
                let s = &mut self.controller.settings;
                match picker.target {
                    Target::Input => {
                        if s.input != value {
                            s.input = value;
                            self.controller.refresh_outputs(true);
                        }
                    }
                    Target::Directory => {
                        s.directory = value;
                        self.controller.refresh_outputs(false);
                    }
                    Target::Output(row) => {
                        s.outputs[row].path = value;
                        s.outputs[row].edited = true;
                    }
                }
            }
            if closed {
                self.picker = None;
            }
            return;
        }
        self.options_dialog(ctx);
    }
}

impl eframe::App for Conv {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.controller.tick();
        if ctx.input(|input| input.viewport().close_requested()) {
            self.controller.request_close();
            if !self.controller.exit_done() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        if self.controller.exit_done() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let locked = self.controller.busy()
            || self.controller.closing
            || self.controller.overwrite.is_some();
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("main form")
                .max_height((ctx.content_rect().height() - 220.0).max(80.0))
                .show(ui, |ui| {
                    ui.add_enabled_ui(!locked, |ui| self.main_controls(ui));
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!locked, egui::Button::new("Convert"))
                    .clicked()
                {
                    self.controller.convert();
                }
                ui.add_enabled(false, egui::Button::new("Abort"))
                    .on_disabled_hover_text("cancel needs a progress mode in MRTKLIB");
                if ui
                    .add_enabled(!locked, egui::Button::new("Options..."))
                    .clicked()
                {
                    self.options = Some(self.controller.settings.options.clone());
                    self.options_error = None;
                }
                for label in ["Plot...", "Process..."] {
                    ui.add_enabled(false, egui::Button::new(label))
                        .on_disabled_hover_text("later");
                }
                if ui
                    .add_enabled(!self.controller.closing, egui::Button::new("Exit"))
                    .clicked()
                {
                    self.controller.request_close();
                    ctx.request_repaint();
                }
            });
            let view = self.controller.view();
            ui.separator();
            ui.add(egui::Label::new(&view.message).truncate())
                .on_hover_text(&view.message);
            egui::ScrollArea::vertical()
                .id_salt("stderr")
                .max_height(100.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if view.dropped > 0 {
                        ui.label(format!("{} older output lines dropped", view.dropped));
                    }
                    for line in view.lines {
                        ui.monospace(line);
                    }
                });
            ui.separator();
            egui::ScrollArea::horizontal()
                .id_salt("command preview")
                .show(ui, |ui| {
                    ui.monospace(self.controller.preview());
                });
        });
        self.dialogs(&ctx);
    }
    fn on_exit(&mut self) {
        self.controller.save();
    }
}

fn time(ui: &mut egui::Ui, label: &str, field: &mut TimeField) {
    ui.vertical(|ui| {
        ui.checkbox(&mut field.enabled, label);
        ui.add_enabled_ui(field.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut field.date)
                        .hint_text("Y/M/D")
                        .desired_width(100.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut field.clock)
                        .hint_text("H:M:S")
                        .desired_width(85.0),
                );
            });
        });
    });
}
fn edits(ui: &mut egui::Ui, values: &mut [String]) {
    let width =
        ((ui.available_width() - (values.len() - 1) as f32 * 4.0) / values.len() as f32).max(30.0);
    for value in values {
        ui.add(egui::TextEdit::singleline(value).desired_width(width));
    }
}
fn fields(ui: &mut egui::Ui, label: &str, values: &mut [String]) {
    ui.horizontal(|ui| {
        ui.add_sized([145.0, 18.0], egui::Label::new(label));
        edits(ui, values);
    });
}
fn disabled_check(ui: &mut egui::Ui, label: &str, mut value: bool, reason: &str) {
    ui.add_enabled(false, egui::Checkbox::new(&mut value, label))
        .on_disabled_hover_text(reason);
}
