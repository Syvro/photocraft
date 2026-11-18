//! Arc-length measurement of a [`Path`]: walking a path at a distance from its start is what
//! Type on a Path needs to place glyphs along a curve (and what future path effects will reuse).
//!
//! Pure geometry over the document path model. The curve is flattened to short chords (the same
//! idea as `photocraft-vector`'s `flatten`, kept here in L1 so text can use it without depending
//! on the rasterizer), then answered by chord lookup. Sampling error is bounded by the flatten
//! tolerance, which is a fraction of a pixel — far below what glyph placement can show.

use crate::{Path, Subpath};
use photocraft_geom::Point;

/// Flattening tolerance in document pixels (max deviation of a chord from the true curve).
pub const TOLERANCE: f64 = 0.25;

/// One measured subpath: a polyline with cumulative arc lengths.
#[derive(Clone, Debug, Default)]
struct Measured {
    /// Polyline points.
    pts: Vec<Point>,
    /// `cum[i]` = distance from the subpath start to `pts[i]`; `cum[0] == 0`.
    cum: Vec<f64>,
    /// Whether the source subpath was closed.
    closed: bool,
}

impl Measured {
    fn len(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Point and unit tangent at distance `s` from the start (clamped to the ends). The tangent
    /// is a local chord across `s`, not the direction of the single (flattened) chord it lies on:
    /// the latter is off by a few degrees on curves and visibly skews glyphs typed on a path.
    fn at(&self, s: f64) -> Option<(Point, Point)> {
        let first = *self.pts.first()?;
        if self.pts.len() == 1 {
            return Some((first, Point::new(1.0, 0.0)));
        }
        let total = self.len();
        let s = if s.is_finite() { s.clamp(0.0, total) } else { 0.0 };
        let point = self.point_at(s)?;
        let d = (TOLERANCE * 4.0).max(0.5);
        let before = self.point_at((s - d).max(0.0))?;
        let after = self.point_at((s + d).min(total))?;
        let (dx, dy) = (after.x - before.x, after.y - before.y);
        let n = dx.hypot(dy);
        let tangent = if n > 1e-9 { Point::new(dx / n, dy / n) } else { Point::new(1.0, 0.0) };
        Some((point, tangent))
    }

    /// Nearest point on the flattened polyline to `p`: distance along the subpath and the
    /// Euclidean distance to that point. Closed subpaths wrap at the end.
    fn nearest(&self, p: Point) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        let chords = self.pts.len().saturating_sub(1);
        for i in 0..=chords {
            if i == chords && !self.closed {
                break;
            }
            let a = *self.pts.get(i)?;
            let b = if i == chords { *self.pts.first()? } else { *self.pts.get(i + 1)? };
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 { (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let q = Point::new(a.x + dx * t, a.y + dy * t);
            let d = (p.x - q.x).hypot(p.y - q.y);
            if best.is_none_or(|(_, prev)| d < prev) {
                let start = self.cum.get(i).copied().unwrap_or(0.0);
                let mut s = start + len2.sqrt() * t;
                let total = self.len();
                if total > 0.0 && s >= total {
                    s -= total;
                }
                best = Some((s, d));
            }
        }
        best
    }

    /// Interpolates the flattened polyline at distance `s` (clamped to the subpath ends).
    fn point_at(&self, s: f64) -> Option<Point> {
        let first = *self.pts.first()?;
        let total = self.len();
        let s = if s.is_finite() { s.clamp(0.0, total) } else { 0.0 };
        let hi = self.cum.len().saturating_sub(1);
        if hi == 0 {
            return Some(first);
        }
        let mut lo = 0usize;
        let mut upper = hi;
        while lo + 1 < upper {
            let mid = (lo + upper) / 2;
            if self.cum.get(mid).copied().unwrap_or(0.0) <= s {
                lo = mid;
            } else {
                upper = mid;
            }
        }
        let p0 = *self.pts.get(lo)?;
        let p1 = *self.pts.get(lo + 1)?;
        let a = self.cum.get(lo).copied().unwrap_or(0.0);
        let b = self.cum.get(lo + 1).copied().unwrap_or(a);
        let t = if b > a { (s - a) / (b - a) } else { 0.0 };
        Some(Point::new(p0.x + (p1.x - p0.x) * t, p0.y + (p1.y - p0.y) * t))
    }
}

/// A path turned into cumulative arc-length tables, one per subpath, in order.
#[derive(Clone, Debug, Default)]
pub struct PathMeasure {
    subpaths: Vec<Measured>,
}

impl PathMeasure {
    /// Flattens `path` for measurement. Empty subpaths (fewer than two knots) are skipped.
    pub fn new(path: &Path) -> Self {
        let subpaths = path.subpaths.iter().filter_map(measure_subpath).collect();
        PathMeasure { subpaths }
    }

    /// Whether there is any measurable geometry.
    pub fn is_empty(&self) -> bool {
        self.subpaths.iter().all(|m| m.len() <= 0.0)
    }

    /// Total length of every subpath.
    pub fn total_len(&self) -> f64 {
        self.subpaths.iter().map(Measured::len).sum()
    }

    /// Point and unit tangent at distance `s` measured from the start of subpath `i` (the first
    /// subpath by default). `None` when there is no such subpath or it has no length.
    pub fn at(&self, subpath: usize, s: f64) -> Option<(Point, Point)> {
        self.subpaths.get(subpath)?.at(s)
    }

    /// Length of subpath `i`.
    pub fn subpath_len(&self, subpath: usize) -> f64 {
        self.subpaths.get(subpath).map_or(0.0, Measured::len)
    }

    /// Number of measured subpaths.
    pub fn subpath_count(&self) -> usize {
        self.subpaths.len()
    }

    /// Nearest point of subpath `i` to `p`: distance along it (to start type there) and the
    /// perpendicular distance (to decide whether a click hit the path at all).
    pub fn nearest(&self, subpath: usize, p: Point) -> Option<(f64, f64)> {
        self.subpaths.get(subpath)?.nearest(p)
    }

    /// Whether subpath `i` was closed in the source path.
    pub fn is_closed(&self, subpath: usize) -> bool {
        self.subpaths.get(subpath).is_some_and(|m| m.closed)
    }
}

/// Flattens one subpath into a polyline with cumulative lengths.
fn measure_subpath(s: &Subpath) -> Option<Measured> {
    let segs = s.segments();
    if segs.is_empty() {
        return None;
    }
    let mut pts: Vec<Point> = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        if i == 0 {
            pts.push(seg[0]);
        }
        flatten_cubic(seg[0], seg[1], seg[2], seg[3], TOLERANCE, &mut pts);
    }
    if pts.len() < 2 {
        return None;
    }
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0);
    for w in pts.windows(2) {
        let d = (w[1].x - w[0].x).hypot(w[1].y - w[0].y);
        let last: f64 = cum[cum.len() - 1];
        cum.push(last + d);
    }
    Some(Measured { pts, cum, closed: s.closed })
}

/// Appends the flattened interior points of the cubic `[p0, c1, c2, p1]` (excluding `p0`).
/// Recursive subdivision until each chord is within `tol`; depth is bounded so pathological input
/// always terminates. Points closer than a hair are dropped to keep the polyline clean.
fn flatten_cubic(p0: Point, c1: Point, c2: Point, p1: Point, tol: f64, out: &mut Vec<Point>) {
    fn recurse(p0: Point, c1: Point, c2: Point, p1: Point, tol: f64, depth: u32, out: &mut Vec<Point>) {
        // Flatness: max distance of the control points from the chord p0→p1.
        let dx = p1.x - p0.x;
        let dy = p1.y - p0.y;
        let chord = dx.hypot(dy);
        let d1 = if chord > 0.0 { ((c1.x - p0.x) * dy - (c1.y - p0.y) * dx).abs() / chord } else { (c1.x - p0.x).hypot(c1.y - p0.y) };
        let d2 = if chord > 0.0 { ((c2.x - p0.x) * dy - (c2.y - p0.y) * dx).abs() / chord } else { (c2.x - p0.x).hypot(c2.y - p0.y) };
        if depth >= 24 || (d1 + d2) <= tol {
            push_point(out, p1);
            return;
        }
        // de Casteljau split at t = 0.5.
        let mid = |a: Point, b: Point| Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
        let a = mid(p0, c1);
        let b = mid(c1, c2);
        let c = mid(c2, p1);
        let d = mid(a, b);
        let e = mid(b, c);
        let f = mid(d, e);
        recurse(p0, a, d, f, tol, depth + 1, out);
        recurse(f, e, c, p1, tol, depth + 1, out);
    }
    recurse(p0, c1, c2, p1, tol, 0, out);
}

/// Pushes `p` unless it coincides with the last point already in `out`.
fn push_point(out: &mut Vec<Point>, p: Point) {
    if let Some(last) = out.last()
        && (p.x - last.x).hypot(p.y - last.y) < 1e-6
    {
        return;
    }
    out.push(p);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_path() -> Path {
        Path::new(vec![Subpath {
            closed: false,
            knots: vec![Knot::corner(0.0, 0.0), Knot::corner(100.0, 0.0)],
            op: crate::PathOp::Combine,
        }])
    }

    use crate::Knot;

    #[test]
    fn straight_line_measures_length_and_direction() {
        let m = PathMeasure::new(&line_path());
        assert!((m.total_len() - 100.0).abs() < 1e-6);

        let (p, t) = m.at(0, 40.0).unwrap();
        assert!((p.x - 40.0).abs() < 1e-6 && p.y.abs() < 1e-6, "{p:?}");
        assert!((t.x - 1.0).abs() < 1e-6 && t.y.abs() < 1e-6, "{t:?}");

        // Clamped at the ends.
        let (p, _) = m.at(0, -5.0).unwrap();
        assert!(p.x.abs() < 1e-9);
        let (p, _) = m.at(0, 500.0).unwrap();
        assert!((p.x - 100.0).abs() < 1e-6);
    }

    #[test]
    fn a_quarter_circle_arc_has_the_right_radius_and_tangent() {
        // Quarter circle of radius 100 in the first quadrant, as cubic Béziers via 4 knots is not
        // exact; use the standard one-segment approximation then check midpoint is near the arc.
        let k = 0.5522847498307936 * 100.0;
        let path = Path::new(vec![Subpath {
            closed: false,
            knots: vec![
                Knot::smooth(Point::new(100.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, k)),
                Knot::smooth(Point::new(0.0, 100.0), Point::new(k, 100.0), Point::new(0.0, 100.0)),
            ],
            op: crate::PathOp::Combine,
        }]);
        let m = PathMeasure::new(&path);
        // True arc length is pi/2 * 100 ≈ 157.08; the cubic approximation is within ~0.03 %.
        assert!((m.total_len() - 157.08).abs() < 0.2, "{}", m.total_len());
        // A point on the curve stays on the circle of radius 100.
        let (p, t) = m.at(0, m.total_len() / 2.0).unwrap();
        assert!(((p.x.hypot(p.y)) - 100.0).abs() < 0.3, "radius {}", p.x.hypot(p.y));
        // Tangent is perpendicular to the radius there.
        let r = Point::new(p.x / p.x.hypot(p.y), p.y / p.x.hypot(p.y));
        assert!((r.x * t.x + r.y * t.y).abs() < 0.01, "tangent not perpendicular: p={p:?} t={t:?}");
    }

    #[test]
    fn empty_and_degenerate_paths_are_safe() {
        assert!(PathMeasure::new(&Path::default()).is_empty());
        let one = Path::new(vec![Subpath { closed: false, knots: vec![Knot::corner(3.0, 3.0)], op: crate::PathOp::Combine }]);
        let m = PathMeasure::new(&one);
        assert!(m.is_empty());
        assert!(m.at(0, 0.0).is_none());
        assert!(m.at(9, 0.0).is_none());
    }

    #[test]
    fn nearest_reports_arc_length_and_click_distance() {
        let m = PathMeasure::new(&line_path());
        let (s, d) = m.nearest(0, Point::new(30.0, 4.0)).unwrap();
        assert!((s - 30.0).abs() < 1e-6, "s {s}");
        assert!((d - 4.0).abs() < 1e-6, "d {d}");
        let (s, d) = m.nearest(0, Point::new(-5.0, 3.0)).unwrap();
        assert!(s.abs() < 1e-6 && (d - 5.8309518948).abs() < 1e-4, "s {s} d {d}");
        assert!(m.nearest(4, Point::new(0.0, 0.0)).is_none());
    }

    #[test]
    fn closed_subpath_reports_closed() {
        let path = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])]);
        let m = PathMeasure::new(&path);
        assert!(m.is_closed(0));
        assert_eq!(m.subpath_count(), 1);
    }
}
