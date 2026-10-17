//! RTKPLOT-like controls; file loading and data transformations stay in Controller.
use crate::{
    controller::Controller,
    data::{self, Sample},
    grid,
    navigation::{self, Area, Bounds, DragMode, Range, Target as ViewTarget, Viewport},
    settings::{ORIGIN_RANGES, ORIGINS, Options, PLOTS, SIZES},
    time,
};
use eframe::egui::{self, Color32};
use egui_plot::{AxisHints, Line, LineStyle, Plot, PlotBounds, PlotPoint, Points};
use mrtklib_egui_common::file_dialog::{FileDialog, Target};
use mrtklib_egui_plotkit::{ecef_to_llh, horizontal_2d, solution_style, statistics};
use mrtklib_egui_supervisor::mask::mask;

pub(crate) const INITIAL_SIZE: [f32; 2] = [1000.0, 760.0];
pub(crate) struct PlotApp {
    controller: Controller,
    dialog: Option<(usize, FileDialog)>,
    discard_dialog: bool,
    options: Option<Options>,
    options_error: Option<String>,
    viewport: Viewport,
    geometry: Option<Geometry>,
    drag: Option<Drag>,
    load_revision: u64,
    revision: u64,
    selected: Option<usize>,
    hovered: Option<usize>,
}
#[derive(Clone, Copy)]
struct Geometry {
    kind: usize,
    canvas: egui::Rect,
    frames: [Option<egui::Rect>; 3],
}
impl Geometry {
    fn wheel_area(self, point: egui::Pos2) -> Area {
        let area = self.area(point);
        if area != Area::Outside || !self.canvas.contains(point) {
            return area;
        }
        // Each panel has time labels below its frame. Extend only the wheel
        // mapping to these bands; RTKPLOT's drag/double-click areas stay intact.
        if let Some(frame) = self
            .frames
            .iter()
            .flatten()
            .rev()
            .find(|frame| point.y > frame.bottom())
            && (frame.left()..=frame.right()).contains(&point.x)
        {
            Area::Time
        } else {
            Area::Outside
        }
    }
    fn area(self, point: egui::Pos2) -> Area {
        if !self.canvas.contains(point) {
            return Area::Outside;
        }
        if self.kind == 0 {
            return Area::Graph(0);
        }
        for (i, frame) in self.frames.iter().enumerate() {
            let Some(frame) = frame else {
                continue;
            };
            if (frame.top()..=frame.bottom()).contains(&point.y) {
                return if point.x < frame.left() {
                    Area::Left(i)
                } else if point.x > frame.right() {
                    Area::Right(i)
                } else {
                    Area::Graph(i)
                };
            }
        }
        if let Some(frame) = self.frames.iter().rev().flatten().next()
            && point.y > frame.bottom()
            && (frame.left()..=frame.right()).contains(&point.x)
        {
            Area::Time
        } else {
            Area::Outside
        }
    }
    fn frame(self, target: ViewTarget) -> egui::Rect {
        let component = match target {
            ViewTarget::Both(i) | ViewTarget::Vertical(i) => i,
            _ => self.frames.iter().position(Option::is_some).unwrap_or(0),
        };
        self.frames[component].unwrap_or(self.canvas)
    }
}
struct Drag {
    target: ViewTarget,
    mode: DragMode,
    start: egui::Pos2,
    bounds: Bounds,
    pixels: [f64; 2],
    button: egui::PointerButton,
}
impl PlotApp {
    pub fn new(ctx: &egui::Context) -> Self {
        let ctx = ctx.clone();
        Self::with_controller(Controller::new(move || ctx.request_repaint()))
    }
    fn with_controller(controller: Controller) -> Self {
        Self {
            controller,
            dialog: None,
            discard_dialog: false,
            options: None,
            options_error: None,
            viewport: Viewport::default(),
            geometry: None,
            drag: None,
            load_revision: 0,
            revision: 0,
            selected: None,
            hovered: None,
        }
    }
    fn update_logic(&mut self, ctx: &egui::Context) {
        self.controller.poll();
        if let Some((slot, dialog)) = &self.dialog {
            let slot = *slot;
            match dialog.poll() {
                Ok(Some(selection)) => {
                    self.dialog = None;
                    if !std::mem::take(&mut self.discard_dialog)
                        && let Some(path) = selection.path
                    {
                        self.controller.open(slot, path);
                    }
                }
                Err(e) => {
                    self.dialog = None;
                    self.controller.error = Some(e);
                }
                Ok(None) => {}
            }
        }
        let paths: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if paths.len() == 1 {
            self.controller
                .open(self.controller.selection.primary(), paths[0].clone());
        } else if paths.len() > 1 {
            self.controller.error = Some("Open one solution file at a time".into());
        }
        if self.load_revision != self.controller.load_revision {
            self.load_revision = self.controller.load_revision;
            self.viewport.loaded(
                self.controller.settings.plot,
                self.controller.settings.options.auto_fit,
                self.controller.settings.options.y_range,
                self.time_extent(),
                &self.controller.samples,
            );
        }
        if self.revision != self.controller.revision {
            self.revision = self.controller.revision;
            self.selected = self
                .controller
                .samples
                .iter()
                .find(|s| {
                    s.source == self.controller.selection.primary()
                        || self.controller.selection.difference
                })
                .map(|s| s.index);
            self.hovered = None;
        }
    }
    fn time_extent(&self) -> Option<Range> {
        let data = self
            .controller
            .dataset_at(self.controller.selection.primary())?;
        let o = &self.controller.settings.options;
        Some(Range::new(
            crate::controller::display_time(data.points.first()?.time, o),
            crate::controller::display_time(data.points.last()?.time, o),
        ))
    }
    fn mouse(&mut self, ui: &egui::Ui) {
        let Some(geometry) = self
            .geometry
            .filter(|g| g.kind == self.controller.settings.plot)
        else {
            return;
        };
        if self.options.is_some() || !ui.is_enabled() {
            self.drag = None;
            return;
        }
        let (pointer, events) = ui.input(|i| (i.pointer.clone(), i.raw.events.clone()));
        let Some(point) = pointer.latest_pos() else {
            self.drag = None;
            return;
        };
        let active_layer = ui.ctx().layer_id_at(point) == Some(ui.layer_id());
        let area = if active_layer {
            geometry.area(point)
        } else {
            Area::Outside
        };
        let track = geometry.kind == 0;
        let wheel_area = if active_layer {
            geometry.wheel_area(point)
        } else {
            Area::Outside
        };
        let line_speed = ui
            .ctx()
            .options(|o| o.input_options.line_scroll_speed.max(1.0));
        let moved = events
            .iter()
            .any(|event| matches!(event, egui::Event::PointerMoved(_)));
        for event in events {
            if let egui::Event::MouseWheel { unit, delta, .. } = event
                && let Some(target) = navigation::wheel_target(track, wheel_area)
            {
                let rect = geometry.frame(target);
                let pixels = [
                    f64::from(rect.width().max(1.0)),
                    f64::from(rect.height().max(1.0)),
                ];
                let notches = f64::from(match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Point => delta.y / line_speed,
                    egui::MouseWheelUnit::Page => delta.y * geometry.canvas.height() / line_speed,
                });
                self.viewport.wheel(target, notches, pixels);
            }
        }
        for (button, mode) in [
            (egui::PointerButton::Primary, DragMode::Pan),
            (egui::PointerButton::Secondary, DragMode::Scale),
        ] {
            let start = pointer.press_origin().unwrap_or(point);
            let start_area = if ui.ctx().layer_id_at(start) == Some(ui.layer_id()) {
                geometry.area(start)
            } else {
                Area::Outside
            };
            if pointer.button_pressed(button)
                && let Some(target) = navigation::drag_target(track, start_area, mode)
            {
                let rect = geometry.frame(target);
                self.drag = Some(Drag {
                    target,
                    mode,
                    start,
                    bounds: self.viewport.bounds(target),
                    pixels: [
                        f64::from(rect.width().max(1.0)),
                        f64::from(rect.height().max(1.0)),
                    ],
                    button,
                });
            }
        }
        if let Some(drag) = &self.drag {
            if moved || pointer.button_released(drag.button) {
                let delta = [
                    f64::from(point.x - drag.start.x),
                    f64::from(point.y - drag.start.y),
                ];
                let bounds = navigation::drag_bounds(
                    drag.bounds,
                    drag.target,
                    drag.mode,
                    delta,
                    drag.pixels,
                );
                self.viewport.set_bounds(drag.target, bounds);
            }
            if pointer.button_released(drag.button) || !pointer.button_down(drag.button) {
                self.drag = None;
            }
        }
        if pointer.button_double_clicked(egui::PointerButton::Primary) && area != Area::Outside {
            let rect = geometry.frame(if track {
                ViewTarget::Track
            } else {
                ViewTarget::Time
            });
            let bounds = self.viewport.bounds(if track {
                ViewTarget::Track
            } else {
                ViewTarget::Time
            });
            let x =
                bounds.x.min + f64::from((point.x - rect.left()) / rect.width()) * bounds.x.span();
            let y = bounds.y.min
                + f64::from((rect.bottom() - point.y) / rect.height()) * bounds.y.span();
            self.viewport.double_click(track, [x, y]);
            self.drag = None;
        }
    }
    fn controls(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                let available = !self.controller.loading && self.dialog.is_none();
                for slot in 0..2 {
                    if ui
                        .add_enabled(
                            available,
                            egui::Button::new(format!("Open Solution-{}...", slot + 1)),
                        )
                        .clicked()
                    {
                        let ctx = ui.ctx().clone();
                        match FileDialog::open(
                            Target::Input,
                            self.controller.last_file(slot),
                            move || ctx.request_repaint(),
                        ) {
                            Ok(dialog) => {
                                self.dialog = Some((slot, dialog));
                                self.discard_dialog = false;
                            }
                            Err(e) => self.controller.error = Some(e),
                        }
                        ui.close();
                    }
                }
                if ui
                    .add_enabled(
                        available && (0..2).any(|slot| self.controller.last_file(slot).is_some()),
                        egui::Button::new("Reload"),
                    )
                    .clicked()
                {
                    self.controller.reload();
                    ui.close();
                }
                if ui.button("Clear").clicked() {
                    self.discard_dialog = self.dialog.is_some();
                    self.controller.clear();
                    ui.close();
                }
            });
            ui.menu_button("Options", |ui| {
                if ui
                    .checkbox(
                        &mut self.controller.settings.options.statistics,
                        "Show Statistics",
                    )
                    .changed()
                {
                    self.controller.persist();
                }
                if ui.button("Options...").clicked() {
                    self.options = Some(self.controller.settings.options.clone());
                    self.options_error = None;
                    ui.close();
                }
            });
            for slot in 0..2 {
                if let Some(path) = self.controller.last_file(slot) {
                    ui.label(mask(
                        &format!("[{}] {}", slot + 1, path.to_string_lossy()),
                        &[],
                    ));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            let before = self.controller.settings.clone();
            combo(ui, "plot type", &mut self.controller.settings.plot, &PLOTS);
            for (button, label) in ["Data 1", "Data 2", "Data 1-2"].iter().enumerate() {
                let selected = if button == 2 {
                    self.controller.selection.difference
                } else {
                    self.controller.selection.data[button]
                };
                if ui
                    .add_enabled(
                        button != 2 || self.controller.difference_available(),
                        egui::Button::new(*label).selected(selected),
                    )
                    .clicked()
                {
                    self.controller.toggle(button);
                }
            }
            ui.add_enabled_ui(self.controller.settings.plot != 0, |ui| {
                for i in 0..3 {
                    ui.toggle_value(
                        &mut self.controller.settings.components[i],
                        format!("Plot{}", i + 1),
                    );
                }
            });
            ui.label("Quality");
            let mut q = usize::from(self.controller.settings.quality);
            combo(
                ui,
                "quality",
                &mut q,
                &[
                    "ALL", "1 FIX", "2 FLOAT", "3 SBAS", "4 DGPS", "5 SINGLE", "6 PPP",
                ],
            );
            self.controller.settings.quality = q as u8;
            if ui.button("Fit Horizontal").clicked() {
                self.viewport.fit_horizontal(
                    self.controller.settings.plot,
                    self.time_extent(),
                    &self.controller.samples,
                );
            }
            if self.controller.settings.plot != 2 && ui.button("Fit Vertical").clicked() {
                self.viewport
                    .fit_range(self.controller.settings.plot, &self.controller.samples);
            }
            if ui
                .add_enabled(
                    self.controller.settings.plot != 2,
                    egui::Button::new("Center Origin"),
                )
                .clicked()
            {
                self.viewport.center_origin(
                    self.controller.settings.plot,
                    self.controller.settings.options.y_range,
                );
            }
            ui.add_enabled_ui(self.controller.settings.plot != 2, |ui| {
                let range = self.controller.settings.options.y_range;
                let text = origin_range_label(range);
                let button = ui
                    .add(egui::Button::new(format!("{text}...")).min_size(egui::vec2(70.0, 0.0)))
                    .on_hover_text("Center Origin range");
                // A plain popup has no ScrollArea: RTKPLOT shows the entire list.
                egui::Popup::menu(&button)
                    .width(button.rect.width())
                    .show(|ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            for (label, metres) in ORIGIN_RANGES {
                                if ui.selectable_label(range == metres, label).clicked() {
                                    self.controller.settings.options.y_range = metres;
                                    self.viewport
                                        .center_origin(self.controller.settings.plot, metres);
                                }
                            }
                        });
                    });
            });
            if before != self.controller.settings {
                if before.quality != self.controller.settings.quality {
                    self.controller.rebuild();
                }
                if before.plot != self.controller.settings.plot {
                    self.controller.rebuild();
                    self.viewport.plot_type_changed(
                        self.controller.settings.plot,
                        self.controller.settings.options.auto_fit,
                        self.controller.settings.options.y_range,
                        &self.controller.samples,
                    );
                    self.drag = None;
                }
                self.controller.persist();
            }
        });
    }
    fn options_dialog(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.options else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("plot options")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Options");
            egui::Grid::new("plot options grid")
                .num_columns(2)
                .spacing([18.0, 8.0])
                .show(ui, |ui| {
                    left_label(ui, "Time Format");
                    let mut value = if draft.week_time {
                        0
                    } else if draft.utc {
                        2
                    } else {
                        1
                    };
                    combo(
                        ui,
                        "time format",
                        &mut value,
                        &["www/ssss", "h:m:s GPST", "h:m:s UTC"],
                    );
                    draft.week_time = value == 0;
                    draft.utc = value == 2;
                    ui.end_row();
                    left_label(ui, "Lat/Lon Format");
                    let mut value = usize::from(draft.dms);
                    combo(ui, "lat lon format", &mut value, &["Degrees", "DMS"]);
                    draft.dms = value == 1;
                    ui.end_row();
                    left_label(ui, "Auto Fit");
                    let mut value = usize::from(draft.auto_fit);
                    combo(ui, "auto fit", &mut value, &["OFF", "ON"]);
                    draft.auto_fit = value == 1;
                    ui.end_row();
                    left_label(ui, "Show Statistics");
                    ui.checkbox(&mut draft.statistics, "");
                    ui.end_row();
                    left_label(ui, "Plot Style");
                    combo(
                        ui,
                        "plot style",
                        &mut draft.style,
                        &["Mark/Line", "Mark", "Line"],
                    );
                    ui.end_row();
                    left_label(ui, "Mark Size");
                    egui::ComboBox::from_id_salt("mark size")
                        .width(180.0)
                        .height(400.0)
                        .selected_text(draft.mark_size.to_string())
                        .show_ui(ui, |ui| {
                            for size in SIZES {
                                ui.selectable_value(&mut draft.mark_size, size, size.to_string());
                            }
                        });
                    ui.end_row();
                    left_label(ui, "Coordinate Origin");
                    combo(ui, "coordinate origin", &mut draft.origin, &ORIGINS);
                    ui.end_row();
                    for (i, label) in ["Latitude (deg)", "Longitude (deg)", "Height (m)"]
                        .iter()
                        .enumerate()
                    {
                        left_label(ui, label);
                        ui.add_enabled(
                            draft.origin == 5,
                            egui::DragValue::new(&mut draft.llh[i])
                                .speed(if i < 2 { 0.000001 } else { 0.1 })
                                .max_decimals(if i < 2 { 9 } else { 4 }),
                        );
                        ui.end_row();
                    }
                    left_label(ui, "Center Origin Y-range ±");
                    ui.add(egui::DragValue::new(&mut draft.y_range).speed(0.1));
                    ui.end_row();
                });
            if let Some(e) = &self.options_error {
                ui.colored_label(Color32::RED, mask(e, &[]));
            }
            ui.horizontal(|ui| {
                ok = ui.button("OK").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel {
            self.options = None;
            self.options_error = None;
        } else if ok {
            match self
                .controller
                .apply_options(self.options.as_ref().unwrap().clone())
            {
                Ok(()) => {
                    self.options = None;
                    self.options_error = None;
                }
                Err(e) => self.options_error = Some(e),
            }
        }
    }
    fn epoch_text(&self) -> String {
        if !self.controller.selection.difference
            && !self.controller.selection.data.iter().any(|s| *s)
        {
            return String::new();
        }
        let primary = self.controller.selection.primary();
        let Some(dataset) = self.controller.dataset_at(primary) else {
            return "Open or drop an MRTKLIB text solution file".into();
        };
        let index = self.hovered.or(self.selected);
        let Some(s) = index.and_then(|index| {
            self.controller.all_samples[primary]
                .iter()
                .find(|s| s.index == index)
        }) else {
            return "No epochs match the quality selection".into();
        };
        let o = &self.controller.settings.options;
        let p = &dataset.points[s.index];
        let values = match self.controller.settings.plot {
            2 => format!(
                "NS={}  AGE={:.2} s  RATIO={:.1}",
                p.satellites, p.age, p.ratio
            ),
            0 if !dataset.relative => format!(
                "{}   E={:.4} N={:.4} U={:.4} m",
                llh_text(ecef_to_llh(p.position), o.dms),
                s.enu[0],
                s.enu[1],
                s.enu[2]
            ),
            _ => format!("E={:.4} N={:.4} U={:.4} m", s.enu[0], s.enu[1], s.enu[2]),
        };
        format!(
            "[{}] {} {}   {values}   Q={} {}",
            primary + 1,
            time::format(
                crate::controller::display_time(p.time, o),
                o.week_time,
                true
            ),
            time_label(o),
            p.quality,
            solution_style(p.quality).0
        )
    }
    fn summary(&self) -> String {
        if !self.controller.selection.difference
            && !self.controller.selection.data.iter().any(|s| *s)
        {
            return String::new();
        }
        let primary = self.controller.selection.primary();
        let Some(d) = self.controller.dataset_at(primary) else {
            return String::new();
        };
        let mut counts = [0; 8];
        let difference;
        let samples = if self.controller.selection.difference {
            difference = data::difference(
                &self.controller.all_samples[0],
                &self.controller.all_samples[1],
                0,
            );
            &difference
        } else {
            &self.controller.all_samples[primary]
        };
        for p in samples {
            counts[usize::from(p.quality)] += 1;
        }
        let n = samples.len();
        let range = if let (Some(start), Some(end)) = (samples.first(), samples.last()) {
            format!(
                "{} – {} {}",
                time::format(start.time, self.controller.settings.options.week_time, true),
                time::format(end.time, self.controller.settings.options.week_time, true),
                time_label(&self.controller.settings.options)
            )
        } else {
            "No valid epochs".into()
        };
        let mut text = format!(
            "[{}] {range}   N={n}   Malformed lines={}",
            primary + 1,
            d.malformed
        );
        let baselines: Vec<_> = d
            .points
            .iter()
            .map(|p| {
                let r = if d.relative {
                    p.position
                } else {
                    std::array::from_fn(|i| p.position[i] - d.base.unwrap_or(p.position)[i])
                };
                r.iter().map(|v| v * v).sum::<f64>().sqrt()
            })
            .collect();
        if let Some(min) = baselines.iter().copied().reduce(f64::min) {
            let max = baselines.iter().copied().reduce(f64::max).unwrap();
            if min + 100.0 < max {
                text.push_str(&format!("   B={:.1}–{:.1} km", min / 1000.0, max / 1000.0));
            } else {
                text.push_str(&format!("   B={:.1} km", min / 1000.0));
            }
        }
        for (q, count) in counts.iter().enumerate() {
            if (1..=6).contains(&q) || *count > 0 {
                text.push_str(&format!(
                    "   Q{q} {}:{count} ({:.1}%)",
                    solution_style(q as u8).0,
                    if n > 0 {
                        100.0 * *count as f64 / n as f64
                    } else {
                        0.0
                    }
                ));
            }
        }
        text
    }
    fn graphs(&mut self, ui: &mut egui::Ui) {
        if (0..2).all(|slot| {
            self.controller
                .dataset_at(slot)
                .is_none_or(|d| d.points.is_empty())
        }) {
            ui.centered_and_justified(|ui| {
                ui.label(if self.controller.loading {
                    "Loading solution..."
                } else {
                    "No solution epochs to plot"
                });
            });
            return;
        }
        let kind = self.controller.settings.plot;
        let enabled = self.controller.settings.components;
        if kind != 2 {
            if self
                .controller
                .dataset_at(self.controller.selection.primary())
                .is_some_and(|d| d.relative)
            {
                ui.label("Origin: file ENU baseline");
            } else if let Ok(origin) = self.controller.origin(&self.controller.settings.options) {
                ui.label(format!(
                    "Origin: {}   {}",
                    ORIGINS[self.controller.settings.options.origin],
                    llh_text(
                        ecef_to_llh(origin.position),
                        self.controller.settings.options.dms
                    )
                ));
            }
        }

        let panels = if kind == 0 {
            vec![0]
        } else {
            (0..3).filter(|i| enabled[*i]).collect::<Vec<_>>()
        };
        if panels.is_empty() {
            ui.label("Enable Plot1, Plot2 or Plot3 to show a component");
            return;
        }
        let height =
            ((ui.available_height() - 6.0 * panels.len() as f32) / panels.len() as f32).max(50.0);
        self.mouse(ui);
        let canvas = ui.available_rect_before_wrap();
        let mut frames = [None; 3];
        for component in panels {
            frames[component] = Some(self.graph(ui, kind, component, height));
        }
        self.geometry = Some(Geometry {
            kind,
            canvas,
            frames,
        });
    }
    fn graph(
        &mut self,
        ui: &mut egui::Ui,
        kind: usize,
        component: usize,
        height: f32,
    ) -> egui::Rect {
        let samples = &self.controller.samples;
        let o = &self.controller.settings.options;
        let system = self
            .controller
            .dataset_at(self.controller.selection.primary())
            .map_or("", |_| time_label(o));
        let values: Vec<[f64; 2]> = samples
            .iter()
            .map(|s| coordinates(s, kind, component))
            .collect();
        let y_label = if kind == 0 {
            "North (m)"
        } else if kind == 1 {
            ["East (m)", "North (m)", "Up (m)"][component]
        } else {
            ["NSat", "Age (s)", "AR ratio"][component]
        };
        let mut plot = Plot::new(("solution plot", kind, component))
            .height(height)
            .show_background(true)
            .show_grid(false)
            .allow_scroll(false)
            .allow_zoom(false)
            .allow_drag(false)
            .allow_axis_zoom_drag(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .auto_bounds(false)
            .grid_spacing(30.0..=30.0)
            .custom_x_axes(vec![AxisHints::new_x().label_spacing(0.0..=30.0)])
            .custom_y_axes(vec![
                AxisHints::new_y()
                    .min_thickness(55.0)
                    .label_spacing(0.0..=30.0),
            ])
            .x_grid_spacer(move |input| grid::axis_marks(input, kind != 0 && !o.week_time, true))
            .y_grid_spacer(move |input| grid::axis_marks(input, false, kind == 0))
            .y_axis_label(y_label)
            .y_axis_formatter(|mark, _| grid::number(mark.value, mark.step_size))
            .label_formatter(|_| None);
        if kind == 0 {
            plot = plot
                .x_axis_label("East (m)")
                .x_axis_formatter(|mark, _| grid::number(mark.value, mark.step_size));
        } else {
            let week = o.week_time;
            plot = plot
                .x_axis_label(format!("Time ({system})"))
                .x_axis_formatter(move |mark, _| {
                    grid::time_label(mark.value, mark.step_size, week)
                });
        }
        let ctx = ui.ctx().clone();
        let viewport = &mut self.viewport;
        let selected = self.selected;
        let response = plot.show(ui, |pui| {
            if kind == 0
                && viewport.resize_track([
                    f64::from(pui.response().rect.width().max(1.0)),
                    f64::from(pui.response().rect.height().max(1.0)),
                ])
            {
                ctx.request_repaint();
            }
            let bounds = viewport.bounds(if kind == 0 {
                ViewTarget::Track
            } else {
                ViewTarget::Both(component)
            });
            pui.set_plot_bounds(PlotBounds::from_min_max(
                [bounds.x.min, bounds.y.min],
                [bounds.x.max, bounds.y.max],
            ));
            pui.add(grid::Grid::new(kind != 0 && !o.week_time));
            for source in 0..3 {
                let start = samples.partition_point(|s| s.source < source);
                let end = samples.partition_point(|s| s.source <= source);
                let series = &samples[start..end];
                let series_values = &values[start..end];
                if o.style != 1 {
                    for segment in data::segments(series, kind != 0) {
                        pui.line(
                            Line::new(
                                format!("trajectory {source}"),
                                series_values[segment].to_vec(),
                            )
                            .width(1.0)
                            .style(line_style(source))
                            .color(Color32::from_gray(192)),
                        );
                    }
                }
                if o.style != 2 {
                    for quality in 0..=7 {
                        let points: Vec<_> = series
                            .iter()
                            .zip(series_values)
                            .filter(|(s, _)| s.quality == quality)
                            .map(|(_, v)| *v)
                            .collect();
                        let [r, g, b] = data::mark_color(source, quality);
                        pui.points(
                            Points::new(solution_style(quality).0, points)
                                .radius(o.mark_size as f32 / 2.0)
                                .color(Color32::from_rgb(r, g, b)),
                        );
                    }
                }
            }
            if let Some(i) = selected.and_then(|index| {
                samples.iter().position(|s| {
                    s.index == index
                        && (s.source == self.controller.selection.primary()
                            || self.controller.selection.difference)
                })
            }) {
                pui.points(
                    Points::new("selected epoch", vec![values[i]])
                        .radius((o.mark_size as f32 / 2.0 + 3.0).max(4.0))
                        .filled(false)
                        .color(Color32::BLACK),
                );
            }
        });
        if response.response.hovered()
            && let Some(pointer) = response.response.hover_pos()
        {
            let nearest = values
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    samples[*i].source == self.controller.selection.primary()
                        || self.controller.selection.difference
                })
                .min_by(|(_, a), (_, b)| {
                    let distance = |p: &&[f64; 2]| {
                        let screen = response
                            .transform
                            .position_from_point(&PlotPoint::new(p[0], p[1]));
                        if kind == 0 {
                            screen.distance_sq(pointer)
                        } else {
                            (screen.x - pointer.x).powi(2)
                        }
                    };
                    distance(a).total_cmp(&distance(b))
                })
                .map(|(i, _)| samples[i].index);
            self.hovered = nearest;
            if response.response.clicked() {
                self.selected = nearest;
            }
        }
        if o.statistics && kind != 2 {
            let mut overlays = Vec::new();
            for source in 0..3 {
                let enu: Vec<_> = samples
                    .iter()
                    .filter(|s| s.source == source)
                    .map(|s| s.enu)
                    .collect();
                if enu.is_empty() {
                    continue;
                }
                let label = if source == 2 {
                    "1-2"
                } else if source == 0 {
                    "1"
                } else {
                    "2"
                };
                let overlay = if kind == 0 {
                    let mut text = format!("[{label}] 2D={:.4} m", horizontal_2d(&enu));
                    for (i, label) in ["E", "N", "U"].iter().enumerate() {
                        let s = statistics(&enu.iter().map(|p| p[i]).collect::<Vec<_>>());
                        text.push_str(&format!(
                            "\n{label} AVE={:.4} STD={:.4} RMS={:.4} m",
                            s.average, s.std, s.rms
                        ));
                    }
                    text
                } else {
                    let s = statistics(&enu.iter().map(|p| p[component]).collect::<Vec<_>>());
                    format!(
                        "[{label}] AVE={:.4}  STD={:.4}  RMS={:.4} m",
                        s.average, s.std, s.rms
                    )
                };
                overlays.push(overlay);
            }
            let overlay = overlays.join("\n\n");
            let rect = response.transform.frame();
            ui.painter().text(
                rect.left_top() + egui::vec2(8.0, 6.0),
                egui::Align2::LEFT_TOP,
                overlay,
                egui::FontId::monospace(12.0),
                Color32::BLACK,
            );
        }
        *response.transform.frame()
    }
    fn draw(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("plot controls").show(ui, |ui| self.controls(ui));
        egui::Panel::bottom("plot status").show(ui, |ui| {
            if let Some(e) = &self.controller.error {
                ui.colored_label(Color32::RED, mask(e, &[]));
            }
            if self.controller.loading {
                ui.label("Loading solution...");
            }
            for slot in 0..2 {
                if let Some(d) = self.controller.dataset_at(slot) {
                    for warning in &d.warnings {
                        ui.colored_label(
                            Color32::DARK_RED,
                            mask(&format!("[{}] {warning}", slot + 1), &[]),
                        );
                    }
                    if slot != self.controller.selection.primary() && d.malformed > 0 {
                        ui.label(format!("[{}] Malformed lines={}", slot + 1, d.malformed));
                    }
                }
            }
            ui.label(self.epoch_text());
            ui.horizontal_wrapped(|ui| {
                for part in self.summary().split("   ") {
                    ui.add(egui::Label::new(part).extend());
                }
            });
        });
        let previous_hover = self.hovered;
        self.hovered = None;
        egui::CentralPanel::default().show(ui, |ui| self.graphs(ui));
        if previous_hover != self.hovered {
            ui.ctx().request_repaint();
        }
        self.options_dialog(ui.ctx());
    }
}
fn line_style(source: usize) -> LineStyle {
    if source == 1 {
        LineStyle::Dashed { length: 1.0 }
    } else {
        LineStyle::Solid
    }
}
fn origin_range_label(range: f64) -> String {
    ORIGIN_RANGES
        .iter()
        .find(|(_, metres)| *metres == range)
        .map_or_else(|| format!("{range}m"), |(label, _)| (*label).into())
}
impl eframe::App for PlotApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_logic(ui.ctx());
        self.draw(ui);
    }
    fn on_exit(&mut self) {
        self.controller.persist();
        if let Some(e) = &self.controller.error {
            eprintln!("{}", mask(e, &[]));
        }
    }
}
fn combo(ui: &mut egui::Ui, id: &str, value: &mut usize, items: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(items[*value])
        .width(180.0)
        .height(400.0)
        .show_ui(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                for (i, label) in items.iter().enumerate() {
                    ui.selectable_value(value, i, *label);
                }
            });
        });
}
fn left_label(ui: &mut egui::Ui, label: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(160.0, 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(160.0);
            ui.label(label);
        },
    );
}

fn coordinates(s: &Sample, kind: usize, component: usize) -> [f64; 2] {
    match kind {
        0 => [s.enu[0], s.enu[1]],
        1 => [s.time, s.enu[component]],
        _ => [s.time, s.nsat[component]],
    }
}

fn llh_text(llh: [f64; 3], dms: bool) -> String {
    let angle = |value: f64| {
        if !dms {
            return format!("{value:.9}°");
        }
        let total = (value.abs() * 3600.0 * 100000.0).round() / 100000.0;
        format!(
            "{}{}°{:02}'{:08.5}\"",
            if value.is_sign_negative() { "-" } else { "" },
            (total / 3600.0) as u32,
            ((total % 3600.0) / 60.0) as u32,
            total % 60.0
        )
    };
    format!("{} {} H={:.4} m", angle(llh[0]), angle(llh[1]), llh[2])
}
#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;

/// Name of the displayed time system (data are held in GPST).
fn time_label(o: &crate::settings::Options) -> &'static str {
    if o.utc && !o.week_time { "UTC" } else { "GPST" }
}
