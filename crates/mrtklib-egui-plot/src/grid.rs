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
}
impl Grid {
    pub fn new(calendar: bool) -> Self {
        Self {
            base: PlotItemBase::new("grid".into()),
            calendar,
        }
    }
}
impl PlotItem for Grid {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
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
