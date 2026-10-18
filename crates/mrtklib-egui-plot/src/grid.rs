//! RTKPLOT grid ticks and solid-zero / broken nonzero grid drawing.
use crate::navigation::Range;
use eframe::egui::{self, Color32, Shape};
use egui_plot::{
    GridInput, GridMark, PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotTransform,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GridStyle {
    Solid,
    Dotted,
}
pub(crate) fn classify(index: i64) -> GridStyle {
    if index == 0 {
        GridStyle::Solid
    } else {
        GridStyle::Dotted
    }
}

pub(crate) fn tick(scale: f64, calendar: bool) -> f64 {
    if calendar {
        const STEPS: [f64; 23] = [
            0.1, 0.2, 0.5, 1.0, 3.0, 6.0, 12.0, 30.0, 60.0, 300.0, 900.0, 1800.0, 3600.0, 7200.0,
            10800.0, 21600.0, 43200.0, 86400.0, 172800.0, 604800.0, 1209600.0, 3024000.0,
            6048000.0,
        ];
        STEPS
            .into_iter()
            .find(|step| *step >= 60.0 * scale)
            .unwrap_or(12096000.0)
    } else {
        let target = 30.0 * scale;
        let order = 10.0_f64.powf(target.log10().floor());
        [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|v| v * order)
            .find(|v| *v >= target)
            .unwrap_or(10.0 * order)
    }
}
pub(crate) fn number(value: f64, step: f64) -> String {
    let decimals = (0.9 - step.log10()).max(0.0) as usize;
    format!("{value:.decimals$}")
}
pub(crate) fn time_label(value: f64, step: f64, week: bool) -> String {
    if week {
        return crate::time::format(value, true, false);
    }
    let text = crate::time::format(value, false, true);
    let (start, width) = if step < 86400.0 {
        (
            11,
            if step < 1.0 {
                10
            } else if step < 60.0 {
                8
            } else {
                5
            },
        )
    } else if step < 86400.0 * 30.0 {
        (5, 5)
    } else {
        (2, 5)
    };
    text[start..start + width].into()
}
fn positions(range: Range, step: f64) -> impl Iterator<Item = (i64, f64)> {
    let first = (range.min / step).ceil() as i64;
    let last = (range.max / step).floor() as i64;
    (first..=last).map(move |index| (index, index as f64 * step))
}
pub(crate) fn axis_marks(input: GridInput, calendar: bool, double_numeric: bool) -> Vec<GridMark> {
    let scale = input.base_step_size / 30.0;
    let mut step = tick(scale, calendar);
    if double_numeric && !calendar && step / scale < 50.0 {
        step *= 2.0;
    }
    positions(Range::new(input.bounds.0, input.bounds.1), step)
        .map(|(_, value)| GridMark {
            value,
            step_size: step,
        })
        .collect()
}

/// A background plot item uses the final transform, so the grid also follows same-frame fits.
pub(crate) struct Grid {
    base: PlotItemBase,
    calendar: bool,
    ground: Option<(usize, bool, f32)>,
}
impl Grid {
    pub fn new(calendar: bool) -> Self {
        Self {
            base: PlotItemBase::new("grid".into()),
            calendar,
            ground: None,
        }
    }
}
impl Grid {
    pub fn ground(format: usize, show: bool, label_size: f32) -> Self {
        Self {
            ground: Some((format, show, label_size)),
            ..Self::new(false)
        }
    }

    fn ground_shapes(
        &self,
        ui: &egui::Ui,
        transform: &PlotTransform,
        shapes: &mut Vec<Shape>,
        format: usize,
        label_size: f32,
    ) {
        let rect = *transform.frame();
        let stroke = egui::Stroke::new(1.0, self.color());
        // DrawTrk's centre + is independent of ShowGLabel, including OFF.
        let center = rect.center();
        for delta in [egui::vec2(10.0, 0.0), egui::vec2(0.0, 10.0)] {
            shapes.push(Shape::line_segment(
                [center - delta, center + delta],
                stroke,
            ));
        }
        if format == 0 {
            return;
        }
        let bounds = transform.bounds();
        let ranges = [
            Range::new(bounds.min()[0], bounds.max()[0]),
            Range::new(bounds.min()[1], bounds.max()[1]),
        ];
        let scales = [
            ranges[0].span() / f64::from(rect.width()),
            ranges[1].span() / f64::from(rect.height()),
        ];
        let steps = scales.map(|s| tick(s, false));
        let origin = egui::pos2(
            transform.position_from_point_x(0.0),
            transform.position_from_point_y(0.0),
        );
        if format <= 2 {
            Self::new(false).shapes(ui, transform, shapes);
        } else {
            let mut dots = egui::epaint::Mesh::default();
            for radius in circle_radii(ranges, steps[0]) {
                circle_dots(transform, radius, self.color(), &mut dots);
            }
            shapes.push(Shape::mesh(dots));
            shapes.push(Shape::line_segment(
                [
                    egui::pos2(origin.x, rect.top()),
                    egui::pos2(origin.x, rect.bottom()),
                ],
                stroke,
            ));
            shapes.push(Shape::line_segment(
                [
                    egui::pos2(rect.left(), origin.y),
                    egui::pos2(rect.right(), origin.y),
                ],
                stroke,
            ));
        }
        shapes.push(Shape::circle_filled(origin, 3.0, self.color()));
        if format == 2 || format == 4 {
            for axis in 0..2 {
                let step = if steps[axis] / scales[axis] < 50.0 {
                    steps[axis] * 2.0
                } else {
                    steps[axis]
                };
                for (index, value) in positions(ranges[axis], step) {
                    if format == 4 && index == 0 {
                        continue;
                    }
                    let text = number(value, step);
                    let galley = ui.painter().layout_no_wrap(
                        text,
                        egui::FontId::proportional(label_size),
                        Color32::BLACK,
                    );
                    let size = galley.size();
                    let (pos, angle) = if axis == 0 {
                        let x = transform.position_from_point_x(value) - size.x / 2.0;
                        let y = if format == 2 {
                            rect.bottom() - size.y - 1.0
                        } else {
                            origin.y + 2.0
                        };
                        (egui::pos2(x, y), 0.0)
                    } else {
                        let y = transform.position_from_point_y(value);
                        if format == 2 {
                            (
                                egui::pos2(rect.left() + 2.0, y + size.x / 2.0),
                                -std::f32::consts::FRAC_PI_2,
                            )
                        } else {
                            (egui::pos2(origin.x + 5.0, y - size.y / 2.0), 0.0)
                        }
                    };
                    shapes.push(Shape::Text(
                        egui::epaint::TextShape::new(pos, galley, Color32::BLACK).with_angle(angle),
                    ));
                }
            }
        }
        shapes.push(Shape::rect_stroke(
            rect,
            0.0,
            stroke,
            egui::StrokeKind::Inside,
        ));
    }
}

// graph.cpp:538–564 includes the floor/ceil boundary rings, even off-origin.
fn circle_radii([x, y]: [Range; 2], step: f64) -> Vec<f64> {
    let corners = [
        x.min.hypot(y.min),
        x.min.hypot(y.max),
        x.max.hypot(y.min),
        x.max.hypot(y.max),
    ];
    let crosses_x = x.min <= 0.0 && x.max >= 0.0;
    let crosses_y = y.min <= 0.0 && y.max >= 0.0;
    let min = match (crosses_x, crosses_y) {
        (true, true) => 0.0,
        (true, false) => y.min.abs().min(y.max.abs()),
        (false, true) => x.min.abs().min(x.max.abs()),
        (false, false) => corners.into_iter().fold(f64::INFINITY, f64::min),
    };
    let max = corners.into_iter().fold(0.0, f64::max);
    let first = (min / step).floor() as i64;
    let last = (max / step).ceil() as i64;
    (first..=last).map(|i| i as f64 * step).collect()
}

/// Sample only visible circle arcs. Work stays proportional to the frame perimeter
/// even when the origin is far outside the viewport and the radii are very large.
fn circle_dots(
    transform: &PlotTransform,
    radius: f64,
    color: Color32,
    mesh: &mut egui::epaint::Mesh,
) {
    if radius <= 0.0 {
        return;
    }
    let bounds = transform.bounds();
    let mut angles = vec![0.0, std::f64::consts::TAU];
    for x in [bounds.min()[0], bounds.max()[0]] {
        if x.abs() <= radius {
            let a = (x / radius).acos();
            angles.extend([a, std::f64::consts::TAU - a]);
        }
    }
    for y in [bounds.min()[1], bounds.max()[1]] {
        if y.abs() <= radius {
            let a = (y / radius).asin();
            angles.extend([
                a.rem_euclid(std::f64::consts::TAU),
                (std::f64::consts::PI - a).rem_euclid(std::f64::consts::TAU),
            ]);
        }
    }
    angles.sort_by(f64::total_cmp);
    let rect = transform.frame();
    let pixels_per_radian = radius
        * (f64::from(rect.width()) / (bounds.max()[0] - bounds.min()[0]))
            .max(f64::from(rect.height()) / (bounds.max()[1] - bounds.min()[1]));
    for pair in angles.windows(2) {
        let middle = (pair[0] + pair[1]) / 2.0;
        let point = [radius * middle.cos(), radius * middle.sin()];
        if point[0] < bounds.min()[0]
            || point[0] > bounds.max()[0]
            || point[1] < bounds.min()[1]
            || point[1] > bounds.max()[1]
        {
            continue;
        }
        let count = ((pair[1] - pair[0]) * pixels_per_radian / 2.0).ceil() as usize;
        for i in 0..count {
            let a = pair[0] + (pair[1] - pair[0]) * i as f64 / count as f64;
            let pos = egui::pos2(
                transform.position_from_point_x(radius * a.cos()).floor(),
                transform.position_from_point_y(radius * a.sin()).floor(),
            );
            let dot = egui::Rect::from_min_size(pos, egui::vec2(1.0, 1.0)).intersect(*rect);
            if dot.is_positive() {
                mesh.add_colored_rect(dot, color);
            }
        }
    }
}
impl PlotItem for Grid {
    fn shapes(&self, ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        if let Some((format, show, label_size)) = self.ground {
            if show {
                self.ground_shapes(ui, transform, shapes, format, label_size);
            }
            return;
        }
        let mut dots = egui::epaint::Mesh::default();
        let bounds = transform.bounds();
        let rect = transform.frame();
        let ranges = [
            Range::new(bounds.min()[0], bounds.max()[0]),
            Range::new(bounds.min()[1], bounds.max()[1]),
        ];
        for (axis, range) in ranges.into_iter().enumerate() {
            let pixels = if axis == 0 {
                rect.width()
            } else {
                rect.height()
            } as f64;
            let step = tick(range.span() / pixels, self.calendar && axis == 0);
            for (index, value) in positions(range, step) {
                let points = if axis == 0 {
                    let x = transform.position_from_point_x(value);
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())]
                } else {
                    let y = transform.position_from_point_y(value);
                    [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)]
                };
                let stroke = egui::Stroke::new(1.0, self.color());
                match classify(index) {
                    GridStyle::Solid => shapes.push(Shape::line_segment(points, stroke)),
                    GridStyle::Dotted => {
                        // Epaint collapses one-point line segments (and tiny rects
                        // optimized into lines). Explicit quads preserve psDot pixels.
                        let length = if axis == 0 {
                            rect.height()
                        } else {
                            rect.width()
                        };
                        for offset in (0..length.ceil() as usize).step_by(2) {
                            let start = if axis == 0 {
                                egui::pos2(points[0].x.floor(), rect.top().ceil() + offset as f32)
                            } else {
                                egui::pos2(rect.left().ceil() + offset as f32, points[0].y.floor())
                            };
                            dots.add_colored_rect(
                                egui::Rect::from_min_size(start, egui::vec2(1.0, 1.0)),
                                self.color(),
                            );
                        }
                    }
                }
            }
        }
        shapes.push(Shape::mesh(dots));
    }
    fn initialize(&mut self, _range: std::ops::RangeInclusive<f64>) {}
    fn color(&self) -> Color32 {
        Color32::from_gray(192)
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }
    fn bounds(&self) -> PlotBounds {
        PlotBounds::NOTHING
    }
    fn base(&self) -> &PlotItemBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
    fn allow_hover(&self) -> bool {
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn five_formats_draw_reference_primitives_and_label_positions() {
        let ctx = egui::Context::default();
        for bounds in [
            PlotBounds::from_min_max([-4.0, -3.0], [4.0, 3.0]),
            PlotBounds::from_min_max([1.0, 2.0], [9.0, 8.0]),
        ] {
            let frame = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(400.0, 300.0));
            let transform = PlotTransform::new(frame, bounds, [false, false]);
            let mut output = ctx.run_ui(Default::default(), |ui| {
                for format in 0..5 {
                    for show in [false, true] {
                        let mut shapes = vec![];
                        Grid::ground(format, show, 9.0).shapes(ui, &transform, &mut shapes);
                        if !show {
                            assert!(shapes.is_empty());
                            continue;
                        }
                        let crosses = shapes
                            .iter()
                            .filter(|s| {
                                matches!(s, Shape::LineSegment { points: [a, b], .. }
                                if (*a - *b).length() == 20.0)
                            })
                            .count();
                        assert_eq!(crosses, 2);
                        let origin_dots = shapes
                            .iter()
                            .filter(|s| matches!(s, Shape::Circle(_)))
                            .count();
                        assert_eq!(origin_dots, usize::from(format > 0));
                        let labels: Vec<_> = shapes
                            .iter()
                            .filter_map(|s| {
                                if let Shape::Text(t) = s {
                                    Some(t)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        assert_eq!(!labels.is_empty(), format == 2 || format == 4);
                        if format == 2 {
                            assert!(
                                labels
                                    .iter()
                                    .any(|t| t.angle == -std::f32::consts::FRAC_PI_2)
                            );
                            assert!(labels.iter().filter(|t| t.angle == 0.0).all(|t| {
                                (t.pos.y + t.galley.size().y + 1.0 - frame.bottom()).abs() < 0.01
                            }));
                        }
                        if format == 4 {
                            assert!(
                                labels
                                    .iter()
                                    .all(|t| t.angle == 0.0 && t.galley.text() != "0")
                            );
                            let origin_y = transform.position_from_point_y(0.0);
                            let origin_x = transform.position_from_point_x(0.0);
                            assert!(labels.iter().all(|t| {
                                (t.pos.y - origin_y - 2.0).abs() < 0.01
                                    || (t.pos.x - origin_x - 5.0).abs() < 0.01
                            }));
                        }
                        if format == 0 {
                            assert_eq!(shapes.len(), 2);
                            continue;
                        }
                        let dots: Vec<_> = shapes
                            .iter()
                            .filter_map(|s| {
                                if let Shape::Mesh(m) = s {
                                    Some(m)
                                } else {
                                    None
                                }
                            })
                            .flat_map(|m| m.vertices.as_chunks::<4>().0.iter())
                            .map(|quad| transform.value_from_position(quad[0].pos))
                            .collect();
                        assert!(!dots.is_empty());
                        let step = tick(8.0 / 400.0, false);
                        let near_tick =
                            |v: f64| (v / step - (v / step).round()).abs() * step < 0.03;
                        if format <= 2 {
                            assert!(dots.iter().all(|p| near_tick(p.x) || near_tick(p.y)));
                        } else {
                            // Actual rendered dots lie on rings, not rectangular grid lines.
                            assert!(dots.iter().all(|p| near_tick(p.x.hypot(p.y))));
                            assert!(dots.iter().any(|p| !near_tick(p.x) && !near_tick(p.y)));
                        }
                    }
                }
            });
            output.textures_delta.clear();
        }
    }
    #[test]
    fn circle_rings_include_boundary_radii_and_use_origin_not_view_center() {
        assert_eq!(
            circle_radii([Range::new(-3.0, 3.0), Range::new(-4.0, 4.0)], 2.0),
            vec![0.0, 2.0, 4.0, 6.0]
        );
        assert_eq!(
            circle_radii([Range::new(-1.0, 1.0), Range::new(3.0, 4.0)], 2.0),
            vec![2.0, 4.0, 6.0]
        );
        assert_eq!(
            circle_radii([Range::new(3.0, 4.0), Range::new(-1.0, 1.0)], 2.0),
            vec![2.0, 4.0, 6.0]
        );
        assert_eq!(
            circle_radii([Range::new(3.0, 4.0), Range::new(4.0, 5.0)], 2.0),
            vec![4.0, 6.0, 8.0]
        );
    }
    #[test]
    fn nonzero_grid_lines_survive_pixel_snapping() {
        let ctx = egui::Context::default();
        let frame = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 300.0));
        let transform = PlotTransform::new(
            frame,
            PlotBounds::from_min_max([1.0, 1.0], [2.0, 2.0]),
            [false, false],
        );
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let mut shapes = Vec::new();
            Grid::new(false).shapes(ui, &transform, &mut shapes);
            ui.painter().extend(shapes);
        });
        output.textures_delta.clear();
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(primitives.iter().any(|p| {
            let egui::epaint::Primitive::Mesh(mesh) = &p.primitive else {
                return false;
            };
            mesh.indices.as_chunks::<3>().0.iter().any(|indices| {
                let [a, b, c] = std::array::from_fn(|i| mesh.vertices[indices[i] as usize]);
                let ab = b.pos - a.pos;
                let ac = c.pos - a.pos;
                [a, b, c].iter().all(|v| v.color == Color32::from_gray(192))
                    && (ab.x * ac.y - ab.y * ac.x).abs() > 0.1
            })
        }));
    }
    #[test]
    fn numeric_and_calendar_labels_follow_reference_precision() {
        assert_eq!(number(0.4, 0.4), "0.4");
        assert_eq!(number(0.05, 0.05), "0.05");
        let t = crate::time::calendar("2024/08/22", "11:05:03.100").unwrap();
        assert_eq!(time_label(t, 300.0, false), "11:05");
        assert_eq!(time_label(t, 1.0, false), "11:05:03");
        assert_eq!(time_label(t, 0.1, false), "11:05:03.1");
        assert_eq!(time_label(t, 86400.0, false), "08/22");
    }
    #[test]
    fn only_coordinate_zero_is_solid() {
        for (index, value) in positions(Range::new(-0.3, 0.3), 0.1) {
            assert_eq!(
                classify(index),
                if value == 0.0 {
                    GridStyle::Solid
                } else {
                    GridStyle::Dotted
                }
            );
        }
        assert_eq!(classify(-1), GridStyle::Dotted);
        assert_eq!(classify(1), GridStyle::Dotted);
    }
    #[test]
    fn ticks_follow_rtkplot_numeric_and_calendar_steps() {
        assert_eq!(tick(0.01, false), 0.5);
        assert_eq!(tick(0.1, false), 5.0);
        assert_eq!(tick(0.01, true), 1.0);
        assert_eq!(tick(1.0, true), 60.0);
        assert_eq!(tick(2.0, true), 300.0);
        let marks = axis_marks(
            GridInput {
                bounds: (-1.0, 1.0),
                base_step_size: 0.3,
            },
            false,
            true,
        );
        assert!(marks.iter().all(|m| m.step_size == 0.5));
    }
}
