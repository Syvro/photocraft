//! Type on a Path: lays text out along a curve instead of a straight baseline.
//!
//! The text engine places glyphs on a normal baseline first (so kerning, shaping and everything
//! else is unchanged), then this module maps each resulting point from *baseline space* onto the
//! path: `x` becomes distance travelled along the curve and `y` becomes the offset along the
//! curve's normal. It mirrors [`crate::warp::Warp`] — a per-point map applied after the glyph
//! transform and before the layer transform — so rasterizing, outlines, hit-testing and carets
//! all agree by construction.
//!
//! Follows Photoshop's model: text runs from where the user clicked (`start`), the whole run can
//! slide (`offset`), the text can hang on either side (`flip`), and it re-flows whenever the path
//! or the text changes (layout is recomputed from scratch).

use photocraft_doc::path_measure::PathMeasure;
use photocraft_doc::text::TextPathBinding;

/// A path prepared for one layout: arc-length sampler plus the binding's start/offset/flip.
#[derive(Clone, Debug)]
pub struct PathMap {
    measure: PathMeasure,
    subpath: usize,
    /// Distance along the path where baseline `x = 0` lands.
    base: f64,
    /// The path's total length along `subpath` (for clamping and overflow tests).
    len: f64,
    flip: bool,
}

impl PathMap {
    /// Prepares `binding`. `None` when the path has no measurable geometry (nothing to draw on).
    pub fn new(binding: &TextPathBinding) -> Option<PathMap> {
        let measure = PathMeasure::new(&binding.path);
        if measure.is_empty() {
            return None;
        }
        let subpath = binding.subpath.min(measure.subpath_count().saturating_sub(1));
        let len = measure.subpath_len(subpath);
        if len <= 0.0 {
            return None;
        }
        Some(PathMap { measure, subpath, base: f64::from(binding.start) + f64::from(binding.offset), len, flip: binding.flip })
    }

    /// Total length of the path being typed on (text-space px).
    pub fn length(&self) -> f64 {
        self.len
    }

    /// Where, along the path, baseline `x = 0` sits.
    pub fn base(&self) -> f64 {
        self.base
    }

    /// Maps a point in baseline space (x = advance along the baseline, y = offset from it, y up
    /// negative as in text space) to text space on the path. `flip` mirrors the cross-axis.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let s = self.base + x;
        // Clamp to the path so glyphs never shoot off the ends; the caller can also test overflow.
        let (p, t) = match self.measure.at(self.subpath, s) {
            Some(v) => v,
            None => return (x, y),
        };
        let (tx, ty) = if self.flip { (-t.x, -t.y) } else { (t.x, t.y) };
        // Text-space y is down (glyph outlines are flipped by the layout), so a glyph above the
        // baseline has negative y. Rotate the tangent 90° to put that glyph on that side of the
        // curve; flipping reverses the tangent, which moves the run to the other side.
        let (nx, ny) = (-ty, tx);
        (p.x + nx * y, p.y + ny * y)
    }

    /// The signed distance from the baseline origin of a glyph at advance `x` to the path end;
    /// positive means the glyph has run past the end of the path.
    pub fn overflow(&self, x: f64) -> f64 {
        (self.base + x) - self.len
    }

    /// Whether the run starting at `base` with `advance` width fits within the path.
    pub fn fits(&self, advance: f64) -> bool {
        self.base >= 0.0 && self.base + advance <= self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::vector::{Knot, Path, Subpath};
    use photocraft_geom::Point;

    fn straight_binding(len: f64) -> TextPathBinding {
        TextPathBinding {
            path: Path::new(vec![Subpath {
                closed: false,
                knots: vec![Knot::corner(0.0, 0.0), Knot::corner(len, 0.0)],
                op: photocraft_doc::vector::PathOp::Combine,
            }]),
            start: 0.0,
            offset: 0.0,
            flip: false,
            subpath: 0,
            source: None,
            source_xform: None,
        }
    }

    #[test]
    fn straight_path_is_identity_on_x_and_lifts_negative_y() {
        let m = PathMap::new(&straight_binding(200.0)).unwrap();
        // On the baseline, x maps straight through.
        let (x, y) = m.apply(50.0, 0.0);
        assert!((x - 50.0).abs() < 1e-6 && y.abs() < 1e-6, "({x},{y})");
        // y = -10 (text above baseline) must land above the curve (negative document y).
        let (_, y) = m.apply(50.0, -10.0);
        assert!((y + 10.0).abs() < 1e-6, "y {y}");
    }

    #[test]
    fn offset_and_start_shift_the_run() {
        let mut b = straight_binding(200.0);
        b.start = 30.0;
        let m = PathMap::new(&b).unwrap();
        let (x, _) = m.apply(0.0, 0.0);
        assert!((x - 30.0).abs() < 1e-6, "x {x}");
        b.offset = 20.0;
        let m = PathMap::new(&b).unwrap();
        let (x, _) = m.apply(0.0, 0.0);
        assert!((x - 50.0).abs() < 1e-6, "x {x}");
    }

    #[test]
    fn flip_mirrors_the_cross_axis() {
        let mut b = straight_binding(100.0);
        b.flip = true;
        let m = PathMap::new(&b).unwrap();
        let (_, y) = m.apply(10.0, -10.0);
        // Flipped: y=-10 lands on the other side (document y = +10, below the line).
        assert!((y - 10.0).abs() < 1e-6, "y {y}");
    }

    #[test]
    fn fits_and_overflow() {
        let m = PathMap::new(&straight_binding(100.0)).unwrap();
        assert!(m.fits(80.0));
        assert!(!m.fits(150.0));
        assert!(m.overflow(50.0) < 0.0);
        assert!(m.overflow(120.0) > 0.0);
    }

    #[test]
    fn empty_path_yields_none() {
        assert!(PathMap::new(&TextPathBinding::default()).is_none());
        let one = TextPathBinding {
            path: Path::new(vec![Subpath { closed: false, knots: vec![Knot::corner(1.0, 1.0)], op: photocraft_doc::vector::PathOp::Combine }]),
            ..Default::default()
        };
        assert!(PathMap::new(&one).is_none());
    }

    #[test]
    fn point_remains_on_a_curved_path() {
        let k = 0.5522847498307936 * 100.0;
        let b = TextPathBinding {
            path: Path::new(vec![Subpath {
                closed: false,
                knots: vec![
                    Knot::smooth(Point::new(100.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, k)),
                    Knot::smooth(Point::new(0.0, 100.0), Point::new(k, 100.0), Point::new(0.0, 100.0)),
                ],
                op: photocraft_doc::vector::PathOp::Combine,
            }]),
            ..Default::default()
        };
        let m = PathMap::new(&b).unwrap();
        // Midway along the quarter circle the baseline point is near radius 100 from the origin.
        let (x, y) = m.apply(m.length() / 2.0, 0.0);
        assert!((x.hypot(y) - 100.0).abs() < 0.5, "radius {}", x.hypot(y));
    }
}
