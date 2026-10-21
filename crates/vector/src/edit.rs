//! Point-level path editing, Photoshop's Direct Selection and Convert Point tools: move anchors,
//! drag direction handles, reshape a segment by dragging it, convert between smooth and corner
//! points, and hit-test a path's parts. Pure geometry on [`Path`]; the engine's `path.*` edit
//! commands and the canvas previews share it, so a preview is exactly what the command commits.
//!
//! Knots are addressed as `[subpath, knot]`. Every edit validates its indices and returns an
//! error message instead of panicking.

use photocraft_doc::{Knot, Path};
use photocraft_geom::Point;

/// One of a knot's two direction handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// The control point of the segment arriving at the knot.
    In,
    /// The control point of the segment leaving the knot.
    Out,
}

impl Handle {
    pub fn parse(s: &str) -> Option<Handle> {
        match s {
            "in" => Some(Handle::In),
            "out" => Some(Handle::Out),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Handle::In => "in",
            Handle::Out => "out",
        }
    }
}

/// The part of a path under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Anchor([usize; 2]),
    Handle([usize; 2], Handle),
    /// The segment leaving knot `[subpath, knot]`, at curve parameter `t`.
    Segment([usize; 2], f64),
}

fn add(p: Point, d: [f64; 2]) -> Point {
    Point::new(p.x + d[0], p.y + d[1])
}

fn dist(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn knot_mut(path: &mut Path, [s, k]: [usize; 2]) -> Result<&mut Knot, String> {
    path.subpaths.get_mut(s).and_then(|sp| sp.knots.get_mut(k)).ok_or_else(|| format!("no knot {k} in subpath {s}"))
}

/// The knot ending the segment that leaves `[s, k]` (wrapping on closed subpaths).
fn segment_end(path: &Path, [s, k]: [usize; 2]) -> Result<[usize; 2], String> {
    let sp = path.subpaths.get(s).ok_or_else(|| format!("no subpath {s}"))?;
    let n = sp.knots.len();
    match k.checked_add(1) {
        Some(next) if k < n && next < n => Ok([s, next]),
        Some(_) if k < n && sp.closed && n > 1 => Ok([s, 0]),
        _ => Err(format!("no segment leaves knot {k} of subpath {s}")),
    }
}

fn handle_mut(k: &mut Knot, h: Handle) -> &mut Point {
    match h {
        Handle::In => &mut k.in_ctrl,
        Handle::Out => &mut k.out_ctrl,
    }
}

fn handle(k: &Knot, h: Handle) -> Point {
    match h {
        Handle::In => k.in_ctrl,
        Handle::Out => k.out_ctrl,
    }
}

fn other(h: Handle) -> Handle {
    match h {
        Handle::In => Handle::Out,
        Handle::Out => Handle::In,
    }
}

/// A smooth knot keeps its handles collinear: point handle `h`'s opposite away from it, keeping
/// the opposite's length (a retracted handle stays retracted, as in Photoshop).
fn keep_smooth(k: &mut Knot, h: Handle) {
    if !k.smooth {
        return;
    }
    let (a, moved) = (k.anchor, handle(k, h));
    let opp = handle_mut(k, other(h));
    let (len, d) = (dist(*opp, a), dist(moved, a));
    if len > 1e-9 && d > 1e-9 {
        *opp = Point::new(a.x - (moved.x - a.x) / d * len, a.y - (moved.y - a.y) / d * len);
    }
}

/// Moves anchors (with their handles) by `d`. Each knot moves once, however often it is listed.
pub fn move_anchors(path: &mut Path, anchors: &[[usize; 2]], d: [f64; 2]) -> Result<(), String> {
    let mut list = anchors.to_vec();
    list.sort_unstable();
    list.dedup();
    for r in &list {
        knot_mut(path, *r)?;
    }
    for r in list {
        let k = knot_mut(path, r)?;
        *k = Knot { anchor: add(k.anchor, d), in_ctrl: add(k.in_ctrl, d), out_ctrl: add(k.out_ctrl, d), smooth: k.smooth };
    }
    Ok(())
}

/// Puts handle `h` of a knot at `to`. A smooth knot turns its other handle to stay collinear,
/// unless `independent` (Convert Point / ⌥), which makes it a corner whose handles move apart.
pub fn move_handle(path: &mut Path, knot: [usize; 2], h: Handle, to: Point, independent: bool) -> Result<(), String> {
    let k = knot_mut(path, knot)?;
    *handle_mut(k, h) = to;
    if independent {
        k.smooth = false;
    }
    keep_smooth(k, h);
    Ok(())
}

/// Convert Point: without `out`, a corner with retracted handles (a click on an anchor); with
/// `out`, a smooth knot whose handles are `out` and its mirror (a drag from an anchor).
pub fn convert_point(path: &mut Path, knot: [usize; 2], out: Option<Point>) -> Result<(), String> {
    let k = knot_mut(path, knot)?;
    let a = k.anchor;
    *k = match out {
        Some(o) => Knot::smooth(a, Point::new(2.0 * a.x - o.x, 2.0 * a.y - o.y), o),
        None => Knot::corner(a.x, a.y),
    };
    Ok(())
}

/// Drags the segment leaving `knot` at curve parameter `t` by `d`. A straight segment (both
/// handles retracted) moves with its two anchors; a curve reshapes so the point at `t` follows
/// the pointer exactly, with the nearer handle moving more. Smooth end knots stay smooth.
pub fn bend_segment(path: &mut Path, knot: [usize; 2], t: f64, d: [f64; 2]) -> Result<(), String> {
    let end = segment_end(path, knot)?;
    let retracted = |k: &Knot, h: Handle| dist(handle(k, h), k.anchor) < 1e-9;
    let a = *knot_mut(path, knot)?;
    let b = *knot_mut(path, end)?;
    if retracted(&a, Handle::Out) && retracted(&b, Handle::In) {
        return move_anchors(path, &[knot, end], d);
    }
    // B(t) moves by 3(1−t)²t·Δc1 + 3(1−t)t²·Δc2. With Δc1 = (1−t)·s·d and Δc2 = t·s·d that is
    // 3t(1−t)((1−t)² + t²)·s·d, so s below makes it exactly d. The ends are clamped: a drag right
    // at an anchor would need an unbounded handle move.
    let t = if t.is_finite() { t.clamp(0.05, 0.95) } else { 0.5 };
    let u = 1.0 - t;
    let s = 1.0 / (3.0 * t * u * (u * u + t * t));
    let (w1, w2) = (u * s, t * s);
    let ka = knot_mut(path, knot)?;
    ka.out_ctrl = add(ka.out_ctrl, [d[0] * w1, d[1] * w1]);
    keep_smooth(ka, Handle::Out);
    let kb = knot_mut(path, end)?;
    kb.in_ctrl = add(kb.in_ctrl, [d[0] * w2, d[1] * w2]);
    keep_smooth(kb, Handle::In);
    Ok(())
}

/// Splits the segment leaving `[s, k]` at curve parameter `t` and inserts a new knot there,
/// exactly on the curve (de Casteljau). The original segment's two halves keep the shape; the
/// new knot's handles are the inner control points of each half, so it starts smooth. This is
/// the Add Anchor Point tool.
pub fn insert_knot(path: &mut Path, [s, k]: [usize; 2], t: f64) -> Result<(), String> {
    if !(0.0..=1.0).contains(&t) || !t.is_finite() {
        return Err("`t` must be between 0 and 1".into());
    }
    // (0, 1) — at an end the "split" degenerates onto an existing knot.
    if t <= 1e-6 || t >= 1.0 - 1e-6 {
        return Err("`t` must be strictly between 0 and 1".into());
    }
    let end = segment_end(path, [s, k])?;
    let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
    let i = k;
    let j = end[1];
    let a = sp.knots.get(i).ok_or_else(|| format!("no knot {i} in subpath {s}"))?;
    let b = sp.knots.get(j).ok_or_else(|| format!("no knot {j} in subpath {s}"))?;
    let (p0, c1, c2, p1) = (a.anchor, a.out_ctrl, b.in_ctrl, b.anchor);
    // de Casteljau: the two inner controls that bound the split.
    let lerp = |u: Point, v: Point| Point::new(u.x + (v.x - u.x) * t, u.y + (v.y - u.y) * t);
    let ab = lerp(p0, c1);
    let bc = lerp(c1, c2);
    let cd = lerp(c2, p1);
    let abc = lerp(ab, bc);
    let bcd = lerp(bc, cd);
    let mid = lerp(abc, bcd);
    let smooth = a.smooth || b.smooth;
    // First half is [p0, ab, abc, mid]; second is [mid, bcd, cd, p1]. Knot i's out handle
    // becomes `ab`, knot j's in handle `cd`; the new knot holds the inner controls abc/bcd.
    sp.knots.get_mut(i).ok_or_else(|| format!("no knot {i} in subpath {s}"))?.out_ctrl = ab;
    sp.knots.get_mut(j).ok_or_else(|| format!("no knot {j} in subpath {s}"))?.in_ctrl = cd;
    let new = Knot { anchor: mid, in_ctrl: abc, out_ctrl: bcd, smooth };
    sp.knots.insert(i + 1, new);
    Ok(())
}

/// Removes knot `k` of subpath `s` (the Delete Anchor Point tool), keeping the shape as close as
/// a single cubic can: the neighbouring tangents are extended by 4/3 to meet the removed point's
/// span. The subpath is dropped if it falls below two knots (three when closed).
pub fn delete_knot(path: &mut Path, [s, k]: [usize; 2]) -> Result<(), String> {
    let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
    let n = sp.knots.len();
    if k >= n {
        return Err(format!("no knot {k} in subpath {s}"));
    }
    let min = if sp.closed { 3 } else { 2 };
    if n < min {
        return Err(format!("a {} subpath needs at least {min} knots", if sp.closed { "closed" } else { "open" }));
    }
    let prev = if k == 0 { if sp.closed { n - 1 } else { usize::MAX } } else { k - 1 };
    let next = if k + 1 < n { k + 1 } else { if sp.closed { 0 } else { usize::MAX } };
    if prev != usize::MAX && next != usize::MAX {
        // Interior (or wrapping on a closed subpath): rebuild one cubic from prev to next.
        let (pa, po) = (sp.knots[prev].anchor, sp.knots[prev].out_ctrl);
        let (na, ni) = (sp.knots[next].anchor, sp.knots[next].in_ctrl);
        let ext = |from: Point, handle: Point| Point::new(from.x + (handle.x - from.x) * 4.0 / 3.0, from.y + (handle.y - from.y) * 4.0 / 3.0);
        let (po, ni) = (ext(pa, po), ext(na, ni));
        // Guard against a degenerate tangent that would throw the curve far away.
        let far = |p: Point| p.x.abs() > 1e6 || p.y.abs() > 1e6 || !p.x.is_finite() || !p.y.is_finite();
        sp.knots[prev].out_ctrl = if far(po) { pa } else { po };
        sp.knots[next].in_ctrl = if far(ni) { na } else { ni };
    }
    // An endpoint of an open subpath just goes; the remaining endpoint keeps its own handles.
    sp.knots.remove(k);
    if sp.knots.len() < if sp.closed { 3 } else { 2 } {
        // Closed subpaths fall back to open when only two knots remain.
        sp.closed = false;
    }
    if sp.knots.len() < 2 {
        path.subpaths.remove(s);
    }
    Ok(())
}

/// Every anchor of subpath `s` (⌥-click with Direct Selection selects the whole subpath).
pub fn subpath_anchors(path: &Path, s: usize) -> Vec<[usize; 2]> {
    path.subpaths.get(s).map_or_else(Vec::new, |sp| (0..sp.knots.len()).map(|k| [s, k]).collect())
}

/// Anchors inside the box `[x0, y0, x1, y1]` (a Direct Selection marquee), in path order.
pub fn anchors_in(path: &Path, [x0, y0, x1, y1]: [f64; 4]) -> Vec<[usize; 2]> {
    let (x0, x1, y0, y1) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    let mut out = Vec::new();
    for (s, sp) in path.subpaths.iter().enumerate() {
        for (k, kn) in sp.knots.iter().enumerate() {
            if (x0..=x1).contains(&kn.anchor.x) && (y0..=y1).contains(&kn.anchor.y) {
                out.push([s, k]);
            }
        }
    }
    out
}

fn eval(seg: &[Point; 4], t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(a * seg[0].x + b * seg[1].x + c * seg[2].x + d * seg[3].x, a * seg[0].y + b * seg[1].y + c * seg[2].y + d * seg[3].y)
}

/// The part of `path` within `tol` of `p`: one of the `handles` shown (they sit on top), else an
/// anchor, else a segment; the nearest wins within each kind.
pub fn hit(path: &Path, p: Point, tol: f64, handles: &[([usize; 2], Handle)]) -> Option<Hit> {
    let nearest = |it: &mut dyn Iterator<Item = (f64, Hit)>| it.filter(|(d, _)| *d <= tol).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, h)| h);
    let knot = |r: [usize; 2]| path.subpaths.get(r[0]).and_then(|sp| sp.knots.get(r[1]));
    let on_handle = nearest(&mut handles.iter().filter_map(|&(r, h)| {
        let k = knot(r)?;
        Some((dist(handle(k, h), p), Hit::Handle(r, h)))
    }));
    if on_handle.is_some() {
        return on_handle;
    }
    let anchors =
        path.subpaths.iter().enumerate().flat_map(|(s, sp)| sp.knots.iter().enumerate().map(move |(k, kn)| (dist(kn.anchor, p), Hit::Anchor([s, k]))));
    if let Some(h) = nearest(&mut anchors.into_iter()) {
        return Some(h);
    }
    // Segments: the nearest of 64 samples, refined once around it (plenty at screen tolerances).
    const N: usize = 64;
    let mut segs = path.subpaths.iter().enumerate().flat_map(|(s, sp)| {
        sp.segments().into_iter().enumerate().map(move |(k, seg)| {
            let best = (0..=N).map(|i| i as f64 / N as f64).map(|t| (dist(eval(&seg, t), p), t)).min_by(|a, b| a.0.total_cmp(&b.0)).unwrap_or((f64::MAX, 0.5));
            let step = 1.0 / N as f64;
            let fine = (0..=16)
                .map(|i| (best.1 - step + 2.0 * step * i as f64 / 16.0).clamp(0.0, 1.0))
                .map(|t| (dist(eval(&seg, t), p), t))
                .fold(best, |a, b| if b.0 < a.0 { b } else { a });
            (fine.0, Hit::Segment([s, k], fine.1))
        })
    });
    nearest(&mut segs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::Subpath;

    fn tri() -> Path {
        Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)])])
    }

    /// A smooth knot at (50, 0) between corners, handles 20 px either side.
    fn curve() -> Path {
        let mut sp = Subpath::polyline(&[(0.0, 50.0), (50.0, 0.0), (100.0, 50.0)]);
        sp.knots[1] = Knot::smooth(Point::new(50.0, 0.0), Point::new(30.0, 0.0), Point::new(70.0, 0.0));
        Path::new(vec![sp])
    }

    fn at(path: &Path, s: usize, k: usize) -> Knot {
        path.subpaths[s].knots[k]
    }

    #[test]
    fn anchors_move_with_their_handles_once_each() {
        let mut p = curve();
        move_anchors(&mut p, &[[0, 1], [0, 1]], [5.0, -2.0]).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!((k.anchor, k.in_ctrl, k.out_ctrl), (Point::new(55.0, -2.0), Point::new(35.0, -2.0), Point::new(75.0, -2.0)));
        assert_eq!(at(&p, 0, 0).anchor, Point::new(0.0, 50.0), "other anchors stay");
    }

    #[test]
    fn bad_indices_are_errors_and_change_nothing() {
        let mut p = tri();
        assert!(move_anchors(&mut p, &[[0, 0], [0, 9]], [1.0, 1.0]).is_err());
        assert_eq!(p, tri(), "validated before any knot moved");
        assert!(move_handle(&mut p, [3, 0], Handle::In, Point::new(0.0, 0.0), false).is_err());
        assert!(bend_segment(&mut p, [0, 7], 0.5, [1.0, 1.0]).is_err());
        assert!(convert_point(&mut p, [usize::MAX, usize::MAX], None).is_err());
        let mut open = curve();
        assert!(bend_segment(&mut open, [0, 2], 0.5, [1.0, 1.0]).is_err(), "an open subpath's last knot starts no segment");
        assert!(bend_segment(&mut open, [0, usize::MAX], 0.5, [1.0, 1.0]).is_err());
    }

    #[test]
    fn smooth_handles_stay_collinear_and_keep_their_length() {
        let mut p = curve();
        move_handle(&mut p, [0, 1], Handle::Out, Point::new(50.0, 30.0), false).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!(k.out_ctrl, Point::new(50.0, 30.0));
        assert!(dist(k.in_ctrl, Point::new(50.0, -20.0)) < 1e-9, "opposite turned, length 20 kept: {:?}", k.in_ctrl);
        assert!(k.smooth);
    }

    #[test]
    fn independent_handles_break_the_knot() {
        let mut p = curve();
        move_handle(&mut p, [0, 1], Handle::In, Point::new(40.0, 20.0), true).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(40.0, 20.0), Point::new(70.0, 0.0), false));
    }

    #[test]
    fn convert_point_retracts_or_pulls_mirrored_handles() {
        let mut p = curve();
        convert_point(&mut p, [0, 1], None).unwrap();
        assert_eq!(at(&p, 0, 1), Knot::corner(50.0, 0.0));
        convert_point(&mut p, [0, 0], Some(Point::new(10.0, 60.0))).unwrap();
        let k = at(&p, 0, 0);
        assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(-10.0, 40.0), Point::new(10.0, 60.0), true));
    }

    #[test]
    fn straight_segments_move_with_both_anchors() {
        let mut p = tri();
        // The closing segment (2 → 0) wraps.
        bend_segment(&mut p, [0, 2], 0.5, [0.0, 10.0]).unwrap();
        assert_eq!((at(&p, 0, 2).anchor, at(&p, 0, 0).anchor, at(&p, 0, 1).anchor), (Point::new(50.0, 90.0), Point::new(0.0, 10.0), Point::new(100.0, 0.0)));
    }

    #[test]
    fn curves_bend_through_the_dragged_point() {
        for t in [0.2, 0.5, 0.8] {
            let mut p = curve();
            let seg = |p: &Path| p.subpaths[0].segments()[1];
            let before = eval(&seg(&p), t);
            bend_segment(&mut p, [0, 1], t, [3.0, 12.0]).unwrap();
            let after = eval(&seg(&p), t);
            assert!(dist(after, Point::new(before.x + 3.0, before.y + 12.0)) < 1e-9, "t={t}: {after:?}");
            let k = at(&p, 0, 1);
            let (vi, vo) = ((k.in_ctrl.x - 50.0, k.in_ctrl.y), (k.out_ctrl.x - 50.0, k.out_ctrl.y));
            assert!((vi.0 * vo.1 - vi.1 * vo.0).abs() < 1e-9 && vi.0 * vo.0 + vi.1 * vo.1 < 0.0, "the smooth knot stayed smooth");
        }
    }

    #[test]
    fn hits_prefer_handles_then_anchors_then_segments() {
        let p = curve();
        let shown = [([0, 1], Handle::Out)];
        assert_eq!(hit(&p, Point::new(71.0, 1.0), 3.0, &shown), Some(Hit::Handle([0, 1], Handle::Out)));
        assert_eq!(hit(&p, Point::new(71.0, 1.0), 3.0, &[]), None, "hidden handles aren't hit");
        assert_eq!(hit(&p, Point::new(49.0, 1.0), 3.0, &shown), Some(Hit::Anchor([0, 1])));
        let Some(Hit::Segment(r, t)) = hit(&tri(), Point::new(50.0, 1.0), 3.0, &[]) else { panic!("segment") };
        assert_eq!(r, [0, 0]);
        assert!((t - 0.5).abs() < 0.02, "{t}");
        assert_eq!(hit(&tri(), Point::new(50.0, 40.0), 3.0, &[]), None);
    }

    #[test]
    fn marquee_and_subpath_selections() {
        let p = tri();
        assert_eq!(anchors_in(&p, [110.0, -5.0, 40.0, 5.0]), vec![[0, 1]]);
        assert_eq!(subpath_anchors(&p, 0), vec![[0, 0], [0, 1], [0, 2]]);
        assert!(subpath_anchors(&p, 4).is_empty());
    }

    #[test]
    fn inserting_a_knot_splits_the_segment_without_moving_the_curve() {
        let mut p = curve();
        let orig = p.clone();
        let seg = |p: &Path| p.subpaths[0].segments()[0];
        insert_knot(&mut p, [0, 0], 0.5).unwrap();
        assert_eq!(p.subpaths[0].knots.len(), 4, "a knot was inserted after the first");
        let k = at(&p, 0, 1);
        let on_curve = eval(&seg(&orig), 0.5);
        assert!(dist(k.anchor, on_curve) < 1e-9, "the anchor sits on the curve: {k:?} vs {on_curve:?}");
        assert!(k.smooth, "the split knot inherits smoothness from its neighbours");
        let halves = p.subpaths[0].segments();
        assert_eq!(halves.len(), 3);
        assert!(dist(eval(&halves[0], 1.0), k.anchor) < 1e-9, "the first half ends at the new anchor");
        // The two halves trace the original curve exactly.
        let full = orig.subpaths[0].segments()[0];
        for i in 0..=10 {
            let t = i as f64 / 10.0;
            let half = if t <= 0.5 { eval(&halves[0], t * 2.0) } else { eval(&halves[1], (t - 0.5) * 2.0) };
            let d = dist(half, eval(&full, t));
            assert!(d < 1e-9, "t={t}: half={half:?} full={:?} d={d}", eval(&full, t));
        }
    }

    #[test]
    fn deleting_an_endpoint_trims_and_deleting_a_lone_subpath_removes_it() {
        let open = Subpath::polyline(&[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]);
        let mut p = Path::new(vec![open.clone()]);
        delete_knot(&mut p, [0, 0]).unwrap();
        assert_eq!(p.subpaths[0].knots.len(), 2);
        assert_eq!(at(&p, 0, 0).anchor, Point::new(50.0, 0.0), "the new first keeps its place");
        delete_knot(&mut p, [0, 1]).unwrap();
        assert!(p.subpaths.is_empty(), "a subpath below two knots is dropped");
        assert!(delete_knot(&mut p, [0, 0]).is_err(), "no subpath left");
        let mut p = Path::new(vec![open]);
        assert!(delete_knot(&mut p, [0, 9]).is_err(), "out of range");
    }

    #[test]
    fn deleting_a_closed_knot_keeps_the_loop_and_reopens_at_two() {
        let mut p = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])]);
        delete_knot(&mut p, [0, 0]).unwrap();
        assert_eq!(p.subpaths[0].knots.len(), 3);
        assert!(p.subpaths[0].closed, "still a triangle");
        // The two neighbours are bridged by extended tangents (corners: straight again).
        assert_eq!(at(&p, 0, 2).anchor, Point::new(0.0, 100.0));
        assert_eq!(at(&p, 0, 0).anchor, Point::new(100.0, 0.0));
        assert_eq!(at(&p, 0, 2).out_ctrl, at(&p, 0, 2).anchor, "corner: retracted");
        delete_knot(&mut p, [0, 0]).unwrap();
        assert!(!p.subpaths[0].closed, "two knots can't stay closed");
        assert_eq!(p.subpaths[0].knots.len(), 2);
    }

    #[test]
    fn inserting_and_deleting_reject_bad_input_without_touching_the_path() {
        let mut p = curve();
        let before = p.clone();
        for t in [-1.0, 2.0, f64::NAN, 0.0, 1.0] {
            assert!(insert_knot(&mut p, [0, 0], t).is_err(), "t={t}");
        }
        assert!(insert_knot(&mut p, [3, 0], 0.5).is_err());
        assert!(insert_knot(&mut p, [0, 9], 0.5).is_err());
        assert_eq!(p, before, "nothing changed");
        let mut tiny = Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0)])]);
        assert!(delete_knot(&mut tiny, [0, 0]).is_ok(), "two knots → one → dropped");
        let mut one = Path::new(vec![Subpath::polyline(&[(0.0, 0.0)])]);
        assert!(delete_knot(&mut one, [0, 0]).is_err(), "nothing to delete");
    }
}
