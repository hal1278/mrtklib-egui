//! RTKPLOT-like controls; file loading and data transformations stay in Controller.
use crate::{
    controller::Controller,
    data::{self, Sample},
    grid,
    navigation::{self, Area, Bounds, DragMode, Range, Target as ViewTarget, Viewport},
    settings::{ORIGIN_RANGES, ORIGINS, Options, PLOTS, SIZES},
    summary, time,
};
use eframe::egui::{self, Color32};
use egui_plot::{AxisHints, Line, LineStyle, Plot, PlotBounds, PlotPoint, Points};
use mrtklib_egui_common::file_dialog::{FileDialog, Target};
use mrtklib_egui_plotkit::{ecef_to_llh, enu_to_ecef, horizontal_2d, solution_style, statistics};
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
    selected: Option<(usize, usize)>,
    hovered: Option<(usize, usize)>,
    cursor: Option<(usize, [f64; 2])>,
    status_width: f32,
    status_measure: Option<(egui::Painter, egui::FontId)>,
    info_open: bool,
    errors: Vec<String>,
    rate_drafts: [String; 2],
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
            cursor: None,
            status_width: INITIAL_SIZE[0],
            status_measure: None,
            info_open: false,
            errors: Vec::new(),
            rate_drafts: Default::default(),
        }
    }
    fn update_logic(&mut self, ctx: &egui::Context) {
        self.remember_error();
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
                .map(|s| (s.source, s.index));
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
                    self.errors.clear();
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
            let paths: Vec<_> = (0..2)
                .filter_map(|slot| self.controller.last_file(slot).map(|p| (slot, p)))
                .collect();
            if !paths.is_empty() {
                let width = (ui.available_width()
                    - ui.spacing().item_spacing.x * (paths.len() - 1) as f32)
                    / paths.len() as f32;
                ui.horizontal(|ui| {
                    for (slot, path) in paths {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 22.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                self.rate_control(ui, slot);
                                let full = mask(&path.to_string_lossy(), &[]);
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let measure = |text: &str| {
                                    ui.painter()
                                        .layout_no_wrap(text.into(), font.clone(), Color32::BLACK)
                                        .size()
                                        .x
                                };
                                let text = summary::elide_path(
                                    &full,
                                    ui.available_width() - 25.0,
                                    measure,
                                );
                                egui::ScrollArea::horizontal()
                                    .id_salt(("file path", slot))
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(format!("[{}] {text}", slot + 1))
                                                .extend(),
                                        )
                                        .on_hover_text(full);
                                    });
                            },
                        );
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let before = self.controller.settings.clone();
            combo(ui, "plot type", &mut self.controller.settings.plot, &PLOTS);
            for (button, label) in ["1", "2", "1-2"].iter().enumerate() {
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
            if self.controller.settings.plot == 1 {
                for i in 0..3 {
                    ui.toggle_value(
                        &mut self.controller.settings.components[i],
                        format!("Plot{}", i + 1),
                    );
                }
                if ui
                    .add_sized(
                        [60.0, ui.spacing().interact_size.y],
                        egui::Button::new(if self.controller.settings.height {
                            "Height"
                        } else {
                            "U"
                        }),
                    )
                    .on_hover_text("Third component: Up / ellipsoidal height")
                    .clicked()
                {
                    self.controller.settings.height = !self.controller.settings.height;
                }
            }
            if ui
                .button("All")
                .on_hover_text("Toggle all qualities")
                .clicked()
            {
                self.controller.settings.qualities.toggle_all();
            }
            for (i, label) in ["FIX", "FLOAT", "SBAS", "DGPS", "SINGLE", "PPP"]
                .into_iter()
                .enumerate()
            {
                let q = i + 1;
                // Only qualities present in the shown data get a button.
                if !self
                    .controller
                    .samples
                    .iter()
                    .any(|s| usize::from(s.quality) == q)
                    && !self.present_quality(q)
                {
                    continue;
                }
                let visible = self.controller.settings.qualities.0[q];
                let [r, g, b] = QUALITY_COLORS[q];
                let color = if visible {
                    Color32::from_rgb(r, g, b)
                } else {
                    Color32::from_gray(51)
                };
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE))
                            .selected(visible)
                            .fill(color),
                    )
                    .on_hover_text(format!(
                        "Q{q}: {}",
                        if visible { "shown" } else { "hidden" }
                    ))
                    .clicked()
                {
                    self.controller.settings.qualities.0[q] = !visible;
                }
            }
            if icon_button(ui, "Fit Horizontal", 0).clicked() {
                self.viewport.fit_horizontal(
                    self.controller.settings.plot,
                    self.time_extent(),
                    &self.controller.samples,
                );
            }
            if self.controller.settings.plot != 2 && icon_button(ui, "Fit Vertical", 1).clicked() {
                self.viewport
                    .fit_range(self.controller.settings.plot, &self.controller.samples);
            }
            if ui
                .add_enabled_ui(self.controller.settings.plot != 2, |ui| {
                    icon_button(ui, "Center Origin", 2)
                })
                .inner
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
                    .add(egui::Button::new(format!("{text}...")))
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
                if before.qualities != self.controller.settings.qualities
                    || before.height != self.controller.settings.height
                {
                    self.controller.rebuild();
                    if before.height != self.controller.settings.height
                        && let Some(range) = navigation::fit_component(
                            self.controller
                                .samples
                                .iter()
                                .filter_map(|s| s.component(2)),
                        )
                    {
                        self.viewport.vertical[2] = range;
                    }
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

    fn rate_control(&mut self, ui: &mut egui::Ui, slot: usize) {
        let text = self.controller.rate_text(slot);
        let button = ui
            .add_enabled(!self.controller.loading, egui::Button::new(&text))
            .on_hover_text("Sample rate: click to override or use estimated");
        button.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Rate {}: {text}", slot + 1),
            )
        });
        if button.clicked() {
            self.rate_drafts[slot] = self
                .controller
                .settings
                .manual_rates
                .iter()
                .find(|rate| Some(rate.path.clone()) == self.controller.last_file(slot))
                .map(|rate| rate.entered.clone().unwrap_or_else(|| rate.hz.to_string()))
                .or_else(|| self.controller.rate(slot).map(|hz| hz.to_string()))
                .unwrap_or_default();
        }
        egui::Popup::menu(&button).show(|ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.label(format!("Solution-{} sample rate", slot + 1));
                ui.horizontal(|ui| {
                    let label = ui.label("Hz");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.rate_drafts[slot])
                            .desired_width(100.0),
                    )
                    .labelled_by(label.id);
                });
                let hz = self.rate_drafts[slot]
                    .parse::<f64>()
                    .ok()
                    .filter(|hz| hz.is_finite() && *hz > 0.0);
                if hz.is_none() && !self.rate_drafts[slot].is_empty() {
                    ui.colored_label(Color32::RED, "Rate must be positive and finite");
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(hz.is_some(), egui::Button::new("Apply override"))
                        .clicked()
                    {
                        if let Err(error) = self
                            .controller
                            .set_override_hz(slot, Some(&self.rate_drafts[slot]))
                        {
                            self.controller.error = Some(error);
                        }
                        ui.close();
                    }
                    if ui.button("Use estimated").clicked() {
                        if let Err(error) = self.controller.set_override_hz(slot, None) {
                            self.controller.error = Some(error);
                        }
                        ui.close();
                    }
                });
            });
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
    #[cfg(test)]
    fn epoch_text(&self) -> String {
        self.cursor_status(self.hovered).0
    }
    /// A quality occurs in a shown dataset (hidden qualities included, so their buttons stay).
    fn present_quality(&self, q: usize) -> bool {
        let sel = self.controller.selection;
        (0..2)
            .filter(|&slot| sel.difference || sel.data[slot])
            .filter_map(|slot| self.controller.dataset_at(slot))
            .any(|d| d.points.iter().any(|p| usize::from(p.quality) == q))
    }
    fn cursor_status(&self, epoch: Option<(usize, usize)>) -> (String, bool) {
        let o = &self.controller.settings.options;
        if let Some((source, index)) = epoch
            && let Some(s) = self
                .controller
                .samples
                .iter()
                .find(|s| (s.source, s.index) == (source, index))
        {
            let label = source_label(source);
            let mut enu = format!("E={:.4} N={:.4} U={:.4} m", s.enu[0], s.enu[1], s.enu[2]);
            if source == 2
                && self.controller.settings.height
                && let Some(height) = s.height
            {
                enu.push_str(&format!("  dH={height:.4} m"));
            }
            let llh = (source < 2)
                .then(|| self.controller.dataset_at(source))
                .flatten()
                .filter(|d| !d.relative)
                .map(|d| llh_text(ecef_to_llh(d.points[index].position), o.dms));
            let prefix = format!(
                "[{label}] {} {}  ",
                time::format(s.time, o.week_time, true),
                time_label(o)
            );
            let suffix = format!("  Q={}", s.quality);
            let (values, both_shown) = if self.controller.settings.plot == 2 {
                let values = format!(
                    "NS={:.0} AGE={:.2} s RATIO={:.1}",
                    s.nsat[0], s.nsat[1], s.nsat[2]
                );
                (values, false)
            } else {
                self.coordinates(&prefix, llh, enu, &suffix)
            };
            return (format!("{prefix}{values}{suffix}"), both_shown);
        }
        if let Some((panel, [x, y])) = self.cursor {
            let text = if self.controller.settings.plot == 0 {
                // Off the points RTKPLOT shows the cursor's latitude and longitude about the
                // origin (`plotinfo.cpp:388-392`); relative or difference data have no position.
                let llh = (!self.controller.selection.difference
                    && self
                        .controller
                        .dataset
                        .as_ref()
                        .is_some_and(|d| !d.relative))
                .then(|| self.controller.origin(o).ok())
                .flatten()
                .map(|origin| {
                    let llh = ecef_to_llh(enu_to_ecef([x, y, 0.0], origin.position));
                    latlon_text(llh, o.dms)
                });
                return self.coordinates("", llh, format!("E={x:.4} N={y:.4} m"), "");
            } else {
                let (label, unit) = if self.controller.settings.plot == 1 {
                    (
                        [
                            "E",
                            "N",
                            if self.controller.settings.height {
                                "H"
                            } else {
                                "U"
                            },
                        ][panel],
                        " m",
                    )
                } else {
                    (
                        ["NS", "AGE", "RATIO"][panel],
                        if panel == 1 { " s" } else { "" },
                    )
                };
                format!(
                    "{} {}  {label}={y:.4}{unit}",
                    time::format(x, o.week_time, true),
                    time_label(o)
                )
            };
            return (text, false);
        }
        (String::new(), false)
    }
    /// Both LLH and ENU when they fit the status width, else the chosen one.
    fn coordinates(
        &self,
        prefix: &str,
        llh: Option<String>,
        enu: String,
        suffix: &str,
    ) -> (String, bool) {
        let Some(llh) = llh else {
            return (enu, false);
        };
        let both = format!("{llh}  {enu}");
        // Measure with the actual status font; no character-count width heuristic.
        if self.status_text_width(&format!("{prefix}{both}{suffix}")) <= self.status_width {
            (both, true)
        } else if self.controller.settings.cursor_llh {
            (llh, false)
        } else {
            (enu, false)
        }
    }
    fn status_text_width(&self, text: &str) -> f32 {
        // Filled from the same layout used by the status panel.
        self.status_measure
            .as_ref()
            .map_or(f32::INFINITY, |(painter, font)| {
                painter
                    .layout_no_wrap(text.into(), font.clone(), Color32::BLACK)
                    .size()
                    .x
            })
    }
    fn summary(&self) -> String {
        let c = &self.controller;
        let line = |label, samples: &[Sample], d, override_hz| {
            summary::line(
                label,
                samples,
                d,
                &c.settings.options,
                c.settings.interval_percentages,
                override_hz,
            )
        };
        if c.selection.difference {
            return c.dataset.as_ref().map_or_else(String::new, |d| {
                line(
                    "1-2",
                    &data::difference(&c.all_samples[0], &c.all_samples[1], 0),
                    d,
                    c.override_hz(0),
                )
            });
        }
        (0..2)
            .filter(|slot| c.selection.data[*slot])
            .filter_map(|slot| {
                c.dataset_at(slot).map(|d| {
                    // Counts and time range do not depend on coordinate-origin availability.
                    line(
                        source_label(slot),
                        &data::samples_with_origin(d, None, 0),
                        d,
                        c.override_hz(slot),
                    )
                })
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn remember_error(&mut self) {
        if let Some(error) = &self.controller.error
            && !self.errors.contains(error)
        {
            self.errors.push(error.clone());
        }
    }
    fn info_items(&self) -> Vec<(bool, String)> {
        let mut items = Vec::new();
        for e in &self.errors {
            items.push((true, e.clone()));
        }
        if let Some(e) = &self.controller.error
            && !self.errors.contains(e)
        {
            items.push((true, e.clone()));
        }
        for slot in 0..2 {
            if let Some(error) = &self.controller.conversion_errors[slot] {
                items.push((true, format!("[{}] {error}", slot + 1)));
            }
            for e in &self.controller.file_errors[slot] {
                items.push((true, format!("[{}] {e}", slot + 1)));
            }
            if let Some(d) = self.controller.dataset_at(slot) {
                if d.duplicates_removed > 0 {
                    items.push((
                        false,
                        format!(
                            "[{}] Removed duplicate epochs={}",
                            slot + 1,
                            d.duplicates_removed
                        ),
                    ));
                }
                if d.malformed > 0 {
                    items.push((
                        false,
                        format!("[{}] Malformed lines={}", slot + 1, d.malformed),
                    ));
                }
                for warning in &d.warnings {
                    items.push((false, format!("[{}] {warning}", slot + 1)));
                }
                if d.relative
                    && self.controller.settings.height
                    && self.controller.settings.plot == 1
                {
                    items.push((false, format!("[{}] Ellipsoidal height unavailable: relative ENU file has no absolute position", slot + 1)));
                }
            }
        }
        items
    }
    fn info_window(&mut self, ctx: &egui::Context) {
        let items = self.info_items();
        egui::Window::new("Plot information")
            .open(&mut self.info_open)
            .default_width(600.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if items.is_empty() {
                        ui.label("No information or errors");
                    }
                    for (error, item) in items {
                        ui.colored_label(
                            if error {
                                Color32::RED
                            } else {
                                ui.visuals().text_color()
                            },
                            mask(&item, &[]),
                        );
                    }
                });
            });
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
        let enabled = if kind == 1 {
            self.controller.settings.components
        } else {
            [true; 3]
        };
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
        let visible: Vec<_> = self
            .controller
            .samples
            .iter()
            .filter(|s| kind != 1 || s.component(component).is_some())
            .cloned()
            .collect();
        let samples = &visible;
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
            [
                "East (m)",
                "North (m)",
                if self.controller.settings.height {
                    "Height (m)"
                } else {
                    "Up (m)"
                },
            ][component]
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
            if let Some(i) =
                selected.and_then(|index| samples.iter().position(|s| (s.source, s.index) == index))
            {
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
            let distance = |p: &[f64; 2]| {
                response
                    .transform
                    .position_from_point(&PlotPoint::new(p[0], p[1]))
                    .distance_sq(pointer)
            };
            let nearest = values
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))
                .filter(|(_, p)| distance(p) <= 36.0)
                .map(|(i, _)| (samples[i].source, samples[i].index));
            self.hovered = nearest;
            let point = response.transform.value_from_position(pointer);
            self.cursor = Some((component, [point.x, point.y]));
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
                    .map(|s| {
                        let mut values = s.enu;
                        if kind == 1 {
                            values[component] = s
                                .component(component)
                                .expect("unavailable components are excluded before plotting");
                        }
                        values
                    })
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
        self.remember_error();
        egui::Panel::bottom("plot status").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let controls = 105.0 + ui.spacing().item_spacing.x;
                self.status_width = (ui.available_width() - controls).max(1.0);
                self.status_measure = Some((
                    ui.painter().clone(),
                    egui::TextStyle::Body.resolve(ui.style()),
                ));
                let (cursor_text, _) = self.cursor_status(self.hovered);
                // Preserve the width decision while the pointer moves to the status button.
                // Decide on a representative epoch so the button does not depend on hovering.
                let representative = self
                    .hovered
                    .or(self.selected)
                    .or_else(|| self.controller.samples.first().map(|s| (s.source, s.index)));
                let both_shown = self.cursor_status(representative).1;
                ui.allocate_ui_with_layout(
                    egui::vec2(self.status_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(self.status_width);
                        egui::ScrollArea::horizontal()
                            .id_salt("status text")
                            .show(ui, |ui| {
                                for line in self.summary().lines() {
                                    ui.add(egui::Label::new(line).extend());
                                }
                                ui.add(egui::Label::new(cursor_text).extend());
                                if self.controller.loading {
                                    ui.label("Loading solution...");
                                }
                            });
                    },
                );
                ui.vertical(|ui| {
                    let label = if self.controller.settings.interval_percentages {
                        "% Interval"
                    } else {
                        "% Epochs"
                    };
                    if ui
                        .add_sized(
                            [90.0, ui.spacing().interact_size.y],
                            egui::Button::new(label),
                        )
                        .on_hover_text("Quality percentages: whole data interval / epochs present")
                        .clicked()
                    {
                        self.controller.settings.interval_percentages =
                            !self.controller.settings.interval_percentages;
                        self.controller.persist();
                    }
                    ui.horizontal(|ui| {
                        // Hidden while both fit. The fit test uses the text column only,
                        // whose width this button column does not change.
                        if self.controller.settings.plot != 2
                            && !both_shown
                            && ui
                                .button(if self.controller.settings.cursor_llh {
                                    "LLH"
                                } else {
                                    "ENU"
                                })
                                .on_hover_text("Cursor coordinates when both do not fit")
                                .clicked()
                        {
                            self.controller.settings.cursor_llh =
                                !self.controller.settings.cursor_llh;
                            self.controller.persist();
                        }
                        let items = self.info_items();
                        if !items.is_empty()
                            && ui
                                .button(egui::RichText::new(format!("⚠ {}", items.len())).color(
                                    if items.iter().any(|(e, _)| *e) {
                                        Color32::RED
                                    } else {
                                        ui.visuals().text_color()
                                    },
                                ))
                                .on_hover_text("Plot information")
                                .clicked()
                        {
                            self.info_open = true;
                        }
                    });
                });
            });
        });
        let previous_hover = self.hovered;
        self.hovered = None;
        let previous_cursor = self.cursor;
        self.cursor = None;
        egui::CentralPanel::default().show(ui, |ui| self.graphs(ui));
        if previous_hover != self.hovered || previous_cursor != self.cursor {
            ui.ctx().request_repaint();
        }
        self.info_window(ui.ctx());
        self.options_dialog(ui.ctx());
    }
}
// rtktrace include/rtktrace/plot/batch.hpp:40–48 (Q0…Q6).
const QUALITY_COLORS: [[u8; 3]; 7] = [
    [192, 192, 192],
    [0, 128, 0],
    [255, 170, 0],
    [255, 0, 255],
    [0, 0, 255],
    [255, 0, 0],
    [0, 128, 128],
];
fn source_label(source: usize) -> &'static str {
    ["1", "2", "1-2"][source]
}
fn icon_button(ui: &mut egui::Ui, label: &str, kind: usize) -> egui::Response {
    let response = ui.add(egui::Button::new("").min_size(egui::vec2(25.0, 22.0)));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let painter = ui.painter();
    let center = response.rect.center();
    let stroke = egui::Stroke::new(1.2, ui.visuals().text_color());
    let axis = if kind == 1 {
        egui::vec2(0.0, 7.0)
    } else {
        egui::vec2(7.0, 0.0)
    };
    let side = egui::vec2(-axis.y, axis.x) * 0.45;
    painter.line_segment([center - axis, center + axis], stroke);
    if kind == 2 {
        painter.line_segment(
            [center - egui::vec2(0.0, 7.0), center + egui::vec2(0.0, 7.0)],
            stroke,
        );
        painter.circle_stroke(center, 3.5, stroke);
    } else {
        for sign in [-1.0, 1.0] {
            let tip = center + axis * sign;
            painter.line_segment([tip, tip - axis * sign * 0.45 + side], stroke);
            painter.line_segment([tip, tip - axis * sign * 0.45 - side], stroke);
            painter.line_segment([tip - side * 1.4, tip + side * 1.4], stroke);
        }
    }
    response.on_hover_text(label)
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
        .width(0.0)
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
        1 => [
            s.time,
            s.component(component)
                .expect("unavailable components are excluded before plotting"),
        ],
        _ => [s.time, s.nsat[component]],
    }
}

fn llh_text(llh: [f64; 3], dms: bool) -> String {
    format!("{} H={:.4} m", latlon_text(llh, dms), llh[2])
}
fn latlon_text(llh: [f64; 3], dms: bool) -> String {
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
    format!("{} {}", angle(llh[0]), angle(llh[1]))
}
#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;

/// Name of the displayed time system (data are held in GPST).
fn time_label(o: &crate::settings::Options) -> &'static str {
    if o.utc && !o.week_time { "UTC" } else { "GPST" }
}
