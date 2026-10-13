//! NVIDIA usd-brep (`BrepArray`) export, written from the published schema
//! (`source/schema/omniSolid/resources/schema.usda` of the usd-brep project, Apache-2.0).
//!
//! The bodies are first written as STEP (so faces are merged and surfaces recognised exactly as in
//! a STEP export) and restated entity by entity: one `def BrepArray` prim per body under `/World`,
//! named after the body (made a valid USD identifier, repeats numbered), with the body's own name in
//! `customData` (`solidcraft:name`, as the dictionary `solidcraft` holding `name`, the only form the
//! text format allows) and its colour, if any, as a constant `primvars:displayColor`.
//! A prim's indices run over its own flat arrays.
//!
//! Geometry follows what NVIDIA's own exporter writes: planes, cylinders, cones, spheres and tori
//! stay analytic (STEP parameterisation, radians), B-spline surfaces and revolved or extruded
//! surfaces become NURBS surfaces, and every edge is a NURBS curve (lines are degree-1 two-point
//! curves, circles and ellipses rational quadratics). No UV trim curves are written. The prim's
//! `brep:extent` holds every control point, so a rational circle's control hull is inside it.
//!
//! Radial-edge topology, as laid out by the schema and the example `Cube.usda`:
//!
//! - A solid brep has a void region first (the infinite one) then the solid region, then one void
//!   region per cavity. The void region's shell lists, for every face of the outer shell, the
//!   faceuse on the side facing away from the material; the solid region lists the material
//!   side of the outer shell, followed by one shell per cavity (its material sides). A cavity's
//!   void region holds the cavity side of that shell. A sheet has one void region whose shell
//!   holds both faceuses of every face. Every face has exactly two faceuses.
//! - A faceuse is `same` when it is on the side the surface normal points to. STEP's face sense
//!   says whether the material-outward direction is the surface's natural normal.
//! - A face stores its loops as seen from the normal side: the outer loop counter-clockwise, inner
//!   loops clockwise, so the face is always on the left of the travel direction. A STEP bound is
//!   reversed when its orientation flag differs from the face's sense.
//! - An edgeuse is `same` when its loop runs along the edge from its first to its second vertex.
//!   An edgeuse record stands for a mated pair of faceuse sides. Walking round the edge by the
//!   right-hand rule (thumb along the edge) the face is entered from its top (normal side) or its
//!   bottom. With the face on the left of the loop, `same` moves along the normal's side, so the
//!   walk enters from the bottom: `same` is `bottomEntry` and `opposite` is `topEntry`, for outer
//!   and inner loops alike.
//! - `edgeuse:nextRadialEUIndex` links the records of one edge into a cycle in walking order. A
//!   manifold edge, and a seam used twice by one face, has two records that point at each other;
//!   an edge of an open sheet has one record that points at itself.
//! - A face on a sphere or torus must close in its parameter box. NVIDIA's SMLib works the trims of
//!   analytic faces out from the 3D edges, and finds no region inside a loop that winds round a
//!   pole without a seam or that is the outside of a hole (the kernel's own sphere and torus are
//!   made of such faces: it read the sphere with a fifth of the volume and the torus with none).
//!   So a loop turning round a pole gets a meridian seam to the pole, used both ways, and a
//!   clockwise hole gets the box as its outer loop (a pole-to-pole meridian on a sphere, a
//!   meridian circle and a parallel circle through one vertex on a torus) and stays as an inner
//!   loop. See `close_periodic_face`.
//! - A cone's `face:range` stops at the apex (SMLib builds a cone only from non-negative radii).
//! - `loop:vertexIndex` is the sentinel 9999999 (loops here always have edgeuses).
//! - A periodic analytic face gets its own frame, turned about the axis so that the face starts
//!   at u = 0; a plane gets its origin at the corner of the face's parameter box. Their
//!   `face:range` is the box of the face's outer loop in the surface parameters (padded a hair).
//!   A NURBS face's range is the whole knot domain.
//! - `brep:intersectTol3d` is the working tolerance (at least the largest gap between an edge and
//!   its analytic surface); `brep:extent` is the bounding box of every vertex and control point,
//!   padded by it.

use std::collections::{HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fmt::Write as _;

use crate::step_in::p21::{self, Entity, Exchange, Param};
use crate::{ExportBody, KernelError, Result, StepHeader};

type V3 = [f64; 3];

/// Most breps, faces or edges in one export (a bound on hostile sizes).
const MAX_ITEMS: usize = 50_000_000;
/// The `loop:vertexIndex` of a loop made of edgeuses.
const NO_VERTEX: u32 = 9_999_999;

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(KernelError::Failed(format!("USD export: {}", msg.into())))
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn dist(a: V3, b: V3) -> f64 {
    norm(sub(a, b))
}
fn unit(a: V3) -> Option<V3> {
    let n = norm(a);
    (n > 1e-300 && n.is_finite()).then(|| mul(a, 1.0 / n))
}
fn finite3(a: V3) -> bool {
    a.iter().all(|x| x.is_finite())
}

/// An angle difference wrapped into (-pi, pi].
fn wrap_pi(a: f64) -> f64 {
    let d = a % TAU;
    if d > PI {
        d - TAU
    } else if d <= -PI {
        d + TAU
    } else {
        d
    }
}

/// An orthonormal frame: origin, x (reference direction), y = z cross x, z (axis).
#[derive(Clone, Copy, Debug)]
struct Frame {
    o: V3,
    x: V3,
    y: V3,
    z: V3,
}

impl Frame {
    fn new(o: V3, z: V3, x: V3) -> Option<Frame> {
        let z = unit(z)?;
        let x = unit(sub(x, mul(z, dot(x, z)))).or_else(|| {
            let a = if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            unit(sub(a, mul(z, dot(a, z))))
        })?;
        Some(Frame { o, x, y: cross(z, x), z })
    }
    fn local(&self, p: V3) -> V3 {
        let d = sub(p, self.o);
        [dot(d, self.x), dot(d, self.y), dot(d, self.z)]
    }
    fn at(&self, a: f64, b: f64, c: f64) -> V3 {
        add(self.o, add(mul(self.x, a), add(mul(self.y, b), mul(self.z, c))))
    }
    /// The frame turned about its axis so that new u = old u - `ang`.
    fn spun(&self, ang: f64) -> Frame {
        let (c, s) = (ang.cos(), ang.sin());
        Frame { o: self.o, x: add(mul(self.x, c), mul(self.y, s)), y: add(mul(self.y, c), mul(self.x, -s)), z: self.z }
    }
}

// ---------------------------------------------------------------------------------------------
// NURBS curves

/// A rational B-spline curve; control points are not pre-weighted.
#[derive(Clone, Debug)]
struct Nurbs {
    order: usize,
    pts: Vec<V3>,
    w: Vec<f64>,
    knots: Vec<f64>,
}

/// The knot span `i` (p <= i < n) with `k[i] <= t < k[i+1]`; the last span for the end of the domain.
fn find_span(n: usize, p: usize, t: f64, k: &[f64]) -> Option<usize> {
    if n <= p || k.len() < n + p + 1 {
        return None;
    }
    let (lo_t, hi_t) = (*k.get(p)?, *k.get(n)?);
    if !(t >= lo_t && t <= hi_t) {
        return None;
    }
    if t >= hi_t {
        let mut i = n - 1;
        while i > p && *k.get(i)? >= hi_t {
            i -= 1;
        }
        return Some(i);
    }
    let (mut lo, mut hi) = (p, n);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if t < *k.get(mid)? {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(lo)
}

/// The `p + 1` non-zero basis functions at `t` in span `span`.
fn basis_funs(span: usize, t: f64, p: usize, k: &[f64]) -> Option<Vec<f64>> {
    let mut nn = vec![0.0; p + 1];
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];
    *nn.first_mut()? = 1.0;
    for j in 1..=p {
        *left.get_mut(j)? = t - *k.get(span + 1 - j)?;
        *right.get_mut(j)? = *k.get(span + j)? - t;
        let mut saved = 0.0;
        for r in 0..j {
            let denom = *right.get(r + 1)? + *left.get(j - r)?;
            let temp = if denom == 0.0 { 0.0 } else { *nn.get(r)? / denom };
            *nn.get_mut(r)? = saved + *right.get(r + 1)? * temp;
            saved = *left.get(j - r)? * temp;
        }
        *nn.get_mut(j)? = saved;
    }
    Some(nn)
}

impl Nurbs {
    fn degree(&self) -> usize {
        self.order.saturating_sub(1)
    }

    /// The parameter interval the curve is defined on.
    fn domain(&self) -> Option<(f64, f64)> {
        Some((*self.knots.get(self.order.checked_sub(1)?)?, *self.knots.get(self.pts.len())?))
    }

    /// Well formed: sizes agree, order >= 2, knots non-decreasing, weights positive, all finite,
    /// and a non-empty domain.
    fn valid(&self) -> bool {
        let n = self.pts.len();
        self.order >= 2
            && self.order <= n
            && self.w.len() == n
            && self.knots.len() == n + self.order
            && self.pts.iter().all(|p| finite3(*p))
            && self.w.iter().all(|w| w.is_finite() && *w > 0.0)
            && self.knots.iter().all(|k| k.is_finite())
            && self.knots.windows(2).all(|k| k[0] <= k[1])
            && self.domain().is_some_and(|(a, b)| b > a)
    }

    fn eval(&self, t: f64) -> Option<V3> {
        let (a, b) = self.domain()?;
        if t.is_nan() {
            return None;
        }
        let t = t.clamp(a, b);
        let p = self.degree();
        let span = find_span(self.pts.len(), p, t, &self.knots)?;
        let nn = basis_funs(span, t, p, &self.knots)?;
        let (mut acc, mut ws) = ([0.0; 3], 0.0);
        for (j, bj) in nn.iter().enumerate() {
            let i = (span + j).checked_sub(p)?;
            let c = bj * *self.w.get(i)?;
            acc = add(acc, mul(*self.pts.get(i)?, c));
            ws += c;
        }
        (ws.abs() > 1e-300).then(|| mul(acc, 1.0 / ws))
    }

    /// The same curve run backwards.
    fn reversed(&self) -> Nurbs {
        let (a, b) = (self.knots.first().copied().unwrap_or(0.0), self.knots.last().copied().unwrap_or(0.0));
        Nurbs {
            order: self.order,
            pts: self.pts.iter().rev().copied().collect(),
            w: self.w.iter().rev().copied().collect(),
            knots: self.knots.iter().rev().map(|k| a + b - k).collect(),
        }
    }

    fn line(a: V3, b: V3) -> Nurbs {
        Nurbs { order: 2, pts: vec![a, b], w: vec![1.0, 1.0], knots: vec![0.0, 0.0, 1.0, 1.0] }
    }

    /// A degree-1 curve through the points, parameter 0 to n-1.
    fn polyline(points: &[V3]) -> Nurbs {
        let n = points.len();
        let mut knots = vec![0.0];
        knots.extend((0..n).map(|i| i as f64));
        knots.push(n.saturating_sub(1) as f64);
        Nurbs { order: 2, pts: points.to_vec(), w: vec![1.0; n], knots }
    }

    /// A rational quadratic arc of the ellipse `o + ra cos t x + rb sin t y` in `f`, from angle
    /// `t0` through `sweep` (negative runs backwards), in quarter-turn pieces; parameter 0 to 1.
    fn arc(f: &Frame, ra: f64, rb: f64, t0: f64, sweep: f64) -> Nurbs {
        let n = ((sweep.abs() / FRAC_PI_2) - 1e-9).ceil().max(1.0) as usize;
        let dt = sweep / n as f64;
        let w1 = (dt / 2.0).cos();
        let pt = |th: f64, s: f64| f.at(ra * th.cos() * s, rb * th.sin() * s, 0.0);
        let (mut pts, mut w) = (vec![pt(t0, 1.0)], vec![1.0]);
        for i in 0..n {
            let a = t0 + dt * i as f64;
            pts.push(pt(a + dt / 2.0, 1.0 / w1));
            w.push(w1);
            pts.push(pt(a + dt, 1.0));
            w.push(1.0);
        }
        let mut knots = vec![0.0; 3];
        for i in 1..n {
            let k = i as f64 / n as f64;
            knots.extend([k, k]);
        }
        knots.extend([1.0; 3]);
        Nurbs { order: 3, pts, w, knots }
    }

    /// The parameter at which the curve is nearest `p` (sampled, then refined).
    fn nearest(&self, p: V3) -> Option<f64> {
        let (a, b) = self.domain()?;
        let n = (self.pts.len() * 16).clamp(64, 4096);
        let mut best = (f64::INFINITY, a);
        for i in 0..=n {
            let t = a + (b - a) * i as f64 / n as f64;
            let d = dist(self.eval(t)?, p);
            if d < best.0 {
                best = (d, t);
            }
        }
        let h = (b - a) / n as f64;
        let (mut lo, mut hi) = ((best.1 - h).max(a), (best.1 + h).min(b));
        for _ in 0..60 {
            let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if dist(self.eval(m1)?, p) < dist(self.eval(m2)?, p) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        Some((lo + hi) / 2.0)
    }
}

// ---------------------------------------------------------------------------------------------
// Reading the STEP exchange

struct S<'a> {
    ex: &'a Exchange,
}

impl<'a> S<'a> {
    fn e(&self, id: u64) -> Result<&'a Entity> {
        match self.ex.get(id) {
            Some(e) => Ok(e),
            None => fail(format!("missing #{id}")),
        }
    }
    fn r(&self, p: Option<&Param>) -> Result<u64> {
        match p.and_then(Param::as_ref_id) {
            Some(r) => Ok(r),
            None => fail("bad reference"),
        }
    }
    fn f(&self, p: Option<&Param>) -> Result<f64> {
        match p.and_then(Param::as_f64) {
            Some(x) => Ok(x),
            None => fail("bad number"),
        }
    }
    fn list(&self, p: Option<&'a Param>) -> Result<&'a [Param]> {
        match p.and_then(Param::as_list) {
            Some(l) => Ok(l),
            None => fail("bad list"),
        }
    }
    fn point(&self, id: u64) -> Result<V3> {
        let e = self.e(id)?;
        let c = self.list(e.params().get(1))?;
        let p = [self.f(c.first())?, self.f(c.get(1))?, self.f(c.get(2))?];
        if finite3(p) { Ok(p) } else { fail("non-finite point") }
    }
    fn dir(&self, id: u64) -> Result<V3> {
        match unit(self.point(id)?) {
            Some(d) => Ok(d),
            None => fail("zero direction"),
        }
    }
    fn vertex(&self, id: u64) -> Result<V3> {
        self.point(self.r(self.e(id)?.params().get(1))?)
    }
    /// AXIS2_PLACEMENT_3D (origin, axis, reference direction) as an orthonormal frame.
    fn frame(&self, id: u64) -> Result<Frame> {
        let p = self.e(id)?.params();
        let o = self.point(self.r(p.get(1))?)?;
        let z = match p.get(2).and_then(Param::as_ref_id) {
            Some(d) => self.dir(d)?,
            None => [0.0, 0.0, 1.0],
        };
        let x = match p.get(3).and_then(Param::as_ref_id) {
            Some(d) => self.dir(d)?,
            None => [1.0, 0.0, 0.0],
        };
        match Frame::new(o, z, x) {
            Some(f) => Ok(f),
            None => fail("degenerate placement"),
        }
    }
}

/// Knots from multiplicities and values.
fn expand(mults: &[Param], knots: &[Param]) -> Result<Vec<f64>> {
    let mut out = Vec::new();
    if mults.len() != knots.len() {
        return fail("knot and multiplicity lists differ");
    }
    for (m, k) in mults.iter().zip(knots) {
        let (Some(m), Some(k)) = (m.as_i64(), k.as_f64()) else { return fail("bad knot") };
        if !(1..=64).contains(&m) {
            return fail("bad knot multiplicity");
        }
        out.extend(std::iter::repeat_n(k, m as usize));
    }
    Ok(out)
}

fn weight_list(p: &Param) -> Result<Vec<f64>> {
    match p.as_list() {
        Some(l) => l.iter().map(|x| x.as_f64().ok_or_else(|| KernelError::Failed("USD export: bad weight".into()))).collect(),
        None => fail("bad weights"),
    }
}

/// A STEP B-spline curve (with knots; rational when it says so).
fn bspline_curve(s: &S, e: &Entity) -> Result<Nurbs> {
    let unsupported = || KernelError::Failed(format!("USD export: not supported yet: curve {}", e.name()));
    let (deg, ctrl, mults, knots) = if e.is_complex() {
        let b = e.record("B_SPLINE_CURVE").ok_or_else(unsupported)?;
        let k = e.record("B_SPLINE_CURVE_WITH_KNOTS").ok_or_else(unsupported)?;
        (b.first(), b.get(1), k.first(), k.get(1))
    } else if e.name() == "B_SPLINE_CURVE_WITH_KNOTS" {
        let p = e.params();
        (p.get(1), p.get(2), p.get(6), p.get(7))
    } else {
        return Err(unsupported());
    };
    let Some(deg) = deg.and_then(Param::as_i64).filter(|d| (1..=32).contains(d)) else { return fail("bad B-spline degree") };
    let pts = s.list(ctrl)?.iter().map(|c| s.r(Some(c)).and_then(|id| s.point(id))).collect::<Result<Vec<_>>>()?;
    let knots = expand(s.list(mults)?, s.list(knots)?)?;
    let w = match e.record("RATIONAL_B_SPLINE_CURVE").and_then(|r| r.first()) {
        Some(p) => weight_list(p)?,
        None => vec![1.0; pts.len()],
    };
    let c = Nurbs { order: deg as usize + 1, pts, w, knots };
    if c.valid() { Ok(c) } else { fail("unusable B-spline curve") }
}

/// Follow trimmed and surface curves to the curve underneath.
fn basis_curve<'a>(s: &S<'a>, mut id: u64) -> Result<&'a Entity> {
    for _ in 0..8 {
        let e = s.e(id)?;
        match e.name() {
            "TRIMMED_CURVE" | "SURFACE_CURVE" | "SEAM_CURVE" | "INTERSECTION_CURVE" | "BOUNDED_SURFACE_CURVE" => id = s.r(e.params().get(1))?,
            _ => return Ok(e),
        }
    }
    fail("curve nesting too deep")
}

/// An edge: the NURBS curve from the first vertex to the second and its parameter range.
struct EdgeG {
    curve: Nurbs,
    range: (f64, f64),
}

/// The geometry of an EDGE_CURVE running from `pa` to `pb`; `same` is the edge's sense on its curve
/// and `closed` says both ends are one vertex.
fn edge_geometry(s: &S, curve_id: u64, same: bool, closed: bool, pa: V3, pb: V3, tol: f64) -> Result<EdgeG> {
    let e = basis_curve(s, curve_id)?;
    let whole = |curve: Nurbs| -> Result<EdgeG> {
        match curve.domain() {
            Some(range) => Ok(EdgeG { curve, range }),
            None => fail("unusable edge curve"),
        }
    };
    match e.name() {
        "LINE" => {
            if closed {
                return fail("a closed line edge");
            }
            whole(Nurbs::line(pa, pb))
        }
        "CIRCLE" | "ELLIPSE" => {
            let p = e.params();
            let f = s.frame(s.r(p.get(1))?)?;
            let (ra, rb) = if e.name() == "CIRCLE" {
                let r = s.f(p.get(2))?;
                (r, r)
            } else {
                (s.f(p.get(2))?, s.f(p.get(3))?)
            };
            if !(ra > 0.0 && rb > 0.0) {
                return fail("a conic without a radius");
            }
            let angle = |q: V3| {
                let l = f.local(q);
                (l[1] / rb).atan2(l[0] / ra)
            };
            let t0 = angle(pa);
            let sweep = if closed {
                if same { TAU } else { -TAU }
            } else if same {
                (angle(pb) - t0).rem_euclid(TAU)
            } else {
                -(t0 - angle(pb)).rem_euclid(TAU)
            };
            let mut c = Nurbs::arc(&f, ra, rb, t0, sweep);
            // The arc ends on the vertices (they lie on the conic to within the tolerance).
            let n = c.pts.len();
            let (Some(first), Some(last)) = (c.pts.first().copied(), c.pts.last().copied()) else { return fail("empty arc") };
            if dist(first, pa) > 100.0 * tol || dist(last, pb) > 100.0 * tol {
                return fail("an arc that misses its vertices");
            }
            if let Some(q) = c.pts.first_mut() {
                *q = pa;
            }
            if let Some(q) = c.pts.get_mut(n.saturating_sub(1)) {
                *q = pb;
            }
            whole(c)
        }
        "POLYLINE" => {
            let pts = s.list(e.params().get(1))?.iter().map(|c| s.r(Some(c)).and_then(|id| s.point(id))).collect::<Result<Vec<_>>>()?;
            if pts.len() < 2 {
                return fail("a polyline of one point");
            }
            let c = Nurbs::polyline(&pts);
            whole(if same { c } else { c.reversed() })
        }
        _ if e.has("B_SPLINE_CURVE") || e.name() == "B_SPLINE_CURVE_WITH_KNOTS" => {
            let c = bspline_curve(s, e)?;
            let c = if same { c } else { c.reversed() };
            let Some((t0, t1)) = c.domain() else { return fail("unusable edge curve") };
            let (Some(c0), Some(c1)) = (c.eval(t0), c.eval(t1)) else { return fail("unusable edge curve") };
            if dist(c0, pa) <= 10.0 * tol && dist(c1, pb) <= 10.0 * tol {
                return Ok(EdgeG { curve: c, range: (t0, t1) });
            }
            // The edge is a part of a longer curve: find where its vertices lie on it.
            let (Some(ta), Some(tb)) = (c.nearest(pa), c.nearest(pb)) else { return fail("unusable edge curve") };
            let (qa, qb) = (c.eval(ta).unwrap_or(pa), c.eval(tb).unwrap_or(pb));
            if dist(qa, pa) > 100.0 * tol || dist(qb, pb) > 100.0 * tol {
                return fail("an edge whose vertices are off its curve");
            }
            if closed {
                return Ok(EdgeG { curve: c, range: (t0, t1) });
            }
            if tb <= ta {
                return fail("an edge that runs against its curve");
            }
            Ok(EdgeG { curve: c, range: (ta, tb) })
        }
        n => fail(format!("not supported yet: edge curve {n}")),
    }
}

// ---------------------------------------------------------------------------------------------
// Surfaces

#[derive(Clone, Debug)]
struct NurbsSurf {
    ou: usize,
    ov: usize,
    nu: usize,
    nv: usize,
    /// Row-major, u the row index.
    pts: Vec<V3>,
    w: Vec<f64>,
    ku: Vec<f64>,
    kv: Vec<f64>,
}

impl NurbsSurf {
    fn valid(&self) -> bool {
        let sorted = |k: &[f64]| k.iter().all(|x| x.is_finite()) && k.windows(2).all(|p| p[0] <= p[1]);
        self.ou >= 2
            && self.ov >= 2
            && self.ou <= self.nu
            && self.ov <= self.nv
            && self.pts.len() == self.nu * self.nv
            && self.w.len() == self.pts.len()
            && self.ku.len() == self.nu + self.ou
            && self.kv.len() == self.nv + self.ov
            && sorted(&self.ku)
            && sorted(&self.kv)
            && self.pts.iter().all(|p| finite3(*p))
            && self.w.iter().all(|w| w.is_finite() && *w > 0.0)
            && self.domain().is_some_and(|d| d[2] > d[0] && d[3] > d[1])
    }
    /// (u0, v0, u1, v1)
    fn domain(&self) -> Option<[f64; 4]> {
        Some([*self.ku.get(self.ou.checked_sub(1)?)?, *self.kv.get(self.ov.checked_sub(1)?)?, *self.ku.get(self.nu)?, *self.kv.get(self.nv)?])
    }
}

#[derive(Clone, Debug)]
enum Surf {
    Nurbs(NurbsSurf),
    Plane(Frame),
    Cylinder(Frame, f64),
    /// Frame at the radius `r`, semi-angle in (0, pi/2).
    Cone(Frame, f64, f64),
    Sphere(Frame, f64),
    Torus(Frame, f64, f64),
}

fn bspline_surface(s: &S, e: &Entity) -> Result<NurbsSurf> {
    let unsupported = || KernelError::Failed(format!("USD export: not supported yet: surface {}", e.name()));
    let (du, dv, ctrl, mu, mv, ku, kv) = if e.is_complex() {
        let b = e.record("B_SPLINE_SURFACE").ok_or_else(unsupported)?;
        let k = e.record("B_SPLINE_SURFACE_WITH_KNOTS").ok_or_else(unsupported)?;
        (b.first(), b.get(1), b.get(2), k.first(), k.get(1), k.get(2), k.get(3))
    } else if e.name() == "B_SPLINE_SURFACE_WITH_KNOTS" {
        let p = e.params();
        (p.get(1), p.get(2), p.get(3), p.get(8), p.get(9), p.get(10), p.get(11))
    } else {
        return Err(unsupported());
    };
    let degree = |p: Option<&Param>| p.and_then(Param::as_i64).filter(|d| (1..=32).contains(d)).map(|d| d as usize + 1);
    let (Some(ou), Some(ov)) = (degree(du), degree(dv)) else { return fail("bad B-spline degree") };
    let rows = s.list(ctrl)?;
    let nu = rows.len();
    let mut nv = 0;
    let mut pts = Vec::new();
    for row in rows {
        let row = s.list(Some(row))?;
        if nv == 0 {
            nv = row.len();
        } else if nv != row.len() {
            return fail("a ragged B-spline surface");
        }
        for c in row {
            pts.push(s.point(s.r(Some(c))?)?);
        }
    }
    let ku = expand(s.list(mu)?, s.list(ku)?)?;
    let kv = expand(s.list(mv)?, s.list(kv)?)?;
    let w = match e.record("RATIONAL_B_SPLINE_SURFACE").and_then(|r| r.first()) {
        Some(p) => {
            let mut w = Vec::new();
            for row in p.as_list().ok_or_else(|| KernelError::Failed("USD export: bad weights".into()))? {
                w.extend(weight_list(row)?);
            }
            w
        }
        None => vec![1.0; pts.len()],
    };
    let surf = NurbsSurf { ou, ov, nu, nv, pts, w, ku, kv };
    if surf.valid() { Ok(surf) } else { fail("unusable B-spline surface") }
}

/// A surface of revolution as a NURBS surface: u turns the profile once about the axis (a nine
/// point rational circle, counter-clockwise about the axis), v runs along the profile.
fn revolution(s: &S, e: &Entity) -> Result<NurbsSurf> {
    let p = e.params();
    let prof = basis_curve(s, s.r(p.get(1))?)?;
    let q = match prof.name() {
        "CIRCLE" | "ELLIPSE" => {
            let f = s.frame(s.r(prof.params().get(1))?)?;
            let ra = s.f(prof.params().get(2))?;
            let rb = if prof.name() == "CIRCLE" { ra } else { s.f(prof.params().get(3))? };
            Nurbs::arc(&f, ra, rb, 0.0, TAU)
        }
        _ if prof.has("B_SPLINE_CURVE") || prof.name() == "B_SPLINE_CURVE_WITH_KNOTS" => bspline_curve(s, prof)?,
        n => return fail(format!("not supported yet: a revolved {n}")),
    };
    if !q.valid() {
        return fail("unusable profile of a revolution");
    }
    let ax = s.e(s.r(p.get(2))?)?;
    let o = s.point(s.r(ax.params().get(1))?)?;
    let t = s.dir(s.r(ax.params().get(2))?)?;
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let mut pts = Vec::new();
    let mut w = Vec::new();
    for k in 0..9 {
        let a = FRAC_PI_2 / 2.0 * k as f64;
        let (c, sn, wk) = if k % 2 == 1 { (a.cos() / h, a.sin() / h, h) } else { (a.cos(), a.sin(), 1.0) };
        for (qj, wj) in q.pts.iter().zip(&q.w) {
            let on_axis = add(o, mul(t, dot(sub(*qj, o), t)));
            let d = sub(*qj, on_axis);
            let r = norm(d);
            let x = if r > 1e-12 { mul(d, 1.0 / r) } else { [0.0; 3] };
            let y = cross(t, x);
            pts.push(add(on_axis, mul(add(mul(x, c), mul(y, sn)), r)));
            w.push(wj * wk);
        }
    }
    let ku = vec![0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0];
    let surf = NurbsSurf { ou: 3, ov: q.order, nu: 9, nv: q.pts.len(), pts, w, ku, kv: q.knots.clone() };
    if surf.valid() { Ok(surf) } else { fail("unusable surface of revolution") }
}

fn surface(s: &S, id: u64) -> Result<Surf> {
    let e = s.e(id)?;
    let p = e.params();
    let frame = |s: &S| s.frame(s.r(p.get(1))?);
    let positive = |x: f64, what: &str| if x.is_finite() && x > 0.0 { Ok(x) } else { fail(format!("{what} is not positive")) };
    match e.name() {
        "PLANE" => Ok(Surf::Plane(frame(s)?)),
        "CYLINDRICAL_SURFACE" => Ok(Surf::Cylinder(frame(s)?, positive(s.f(p.get(2))?, "a cylinder radius")?)),
        "SPHERICAL_SURFACE" => Ok(Surf::Sphere(frame(s)?, positive(s.f(p.get(2))?, "a sphere radius")?)),
        "TOROIDAL_SURFACE" => {
            Ok(Surf::Torus(frame(s)?, positive(s.f(p.get(2))?, "a torus major radius")?, positive(s.f(p.get(3))?, "a torus minor radius")?))
        }
        "CONICAL_SURFACE" => {
            let f = frame(s)?;
            let (r, a) = (s.f(p.get(2))?, s.f(p.get(3))?);
            if !(r >= 0.0 && a.abs() < FRAC_PI_2 - 1e-9) {
                return fail("a cone without a usable radius or angle");
            }
            if a == 0.0 {
                return Ok(Surf::Cylinder(f, positive(r, "a cone radius")?));
            }
            if a > 0.0 {
                return Ok(Surf::Cone(f, r, a));
            }
            // Same cone with the axis reversed and both parameters negated (the normal stays put).
            let g = Frame { o: f.o, x: f.x, y: mul(f.y, -1.0), z: mul(f.z, -1.0) };
            Ok(Surf::Cone(g, r, -a))
        }
        "SURFACE_OF_REVOLUTION" => Ok(Surf::Nurbs(revolution(s, e)?)),
        _ if e.has("B_SPLINE_SURFACE") || e.name() == "B_SPLINE_SURFACE_WITH_KNOTS" => Ok(Surf::Nurbs(bspline_surface(s, e)?)),
        n => fail(format!("not supported yet: surface {n}")),
    }
}

// ---------------------------------------------------------------------------------------------
// One brep, restated

struct EdgeM {
    curve: Nurbs,
    range: (f64, f64),
    /// Start and end vertex (the curve runs from the first to the second).
    v: (usize, usize),
}

struct Face {
    surf: Surf,
    /// u0, v0, u1, v1
    range: [f64; 4],
    /// Loops as seen from the normal side (outer first): (edge, runs along the edge).
    loops: Vec<Vec<(usize, bool)>>,
    /// The direction away from the material is the surface's natural normal.
    outward_natural: bool,
    shell: usize,
}

struct Model {
    verts: Vec<V3>,
    edges: Vec<EdgeM>,
    faces: Vec<Face>,
    shells: usize,
    /// No material: one shell of open faces (or a closed surface).
    sheet: bool,
    /// The largest distance from an edge to the analytic surface of a face using it.
    gap: f64,
}

/// Points along an edge in the direction a loop runs it.
fn sample_edge(e: &EdgeM, forward: bool) -> Option<Vec<V3>> {
    let n = if e.curve.order == 2 && e.curve.pts.len() == 2 { 1 } else { (e.curve.pts.len() * 16).clamp(32, 512) };
    let (a, b) = e.range;
    let mut v = (0..=n).map(|i| e.curve.eval(a + (b - a) * i as f64 / n as f64)).collect::<Option<Vec<_>>>()?;
    if !forward {
        v.reverse();
    }
    Some(v)
}

/// Distance of a point from the analytic surface (0 for NURBS).
fn surface_gap(surf: &Surf, p: V3) -> f64 {
    match surf {
        Surf::Nurbs(_) => 0.0,
        Surf::Plane(f) => f.local(p)[2].abs(),
        Surf::Cylinder(f, r) => {
            let l = f.local(p);
            (l[0].hypot(l[1]) - r).abs()
        }
        Surf::Cone(f, r, a) => {
            let l = f.local(p);
            (l[0].hypot(l[1]) - (r + l[2] * a.tan())).abs() * a.cos()
        }
        Surf::Sphere(f, r) => (norm(f.local(p)) - r).abs(),
        Surf::Torus(f, big, small) => {
            let l = f.local(p);
            ((l[0].hypot(l[1]) - big).hypot(l[2]) - small).abs()
        }
    }
}

/// Angles of a sample sequence made continuous (a gap in the sequence, `None`, keeps the angle);
/// also the net turn over the closed sequence, in whole turns.
fn unwrap_angles(raw: &[Option<f64>]) -> (Vec<f64>, f64) {
    let (mut out, mut last, mut cur, mut first) = (Vec::new(), None::<f64>, 0.0, None::<f64>);
    for a in raw.iter().flatten() {
        match last {
            None => {
                cur = *a;
                first = Some(*a);
            }
            Some(l) => cur += wrap_pi(a - l),
        }
        last = Some(*a);
        out.push(cur);
    }
    let start = out.first().copied().unwrap_or(0.0);
    let closing = match (first, last) {
        (Some(f), Some(l)) => wrap_pi(f - l),
        _ => 0.0,
    };
    (out.clone(), ((out.last().copied().unwrap_or(0.0) - start + closing) / TAU).round())
}

fn bounds(v: &[f64]) -> Option<(f64, f64)> {
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

/// The face's parameter box from its outer loop's samples. Periodic surfaces get a frame turned so
/// that the box starts at u = 0 and planes an origin at the box corner. `seam`: the outer loop
/// runs an edge both ways.
fn fit_range(surf: &mut Surf, outer: &[V3], seam: bool, tol: f64) -> Result<[f64; 4]> {
    if let Surf::Nurbs(n) = surf {
        return match n.domain() {
            Some(d) => Ok(d),
            None => fail("a surface without a domain"),
        };
    }
    let pad_a = 1e-4;
    let linear = |lo: f64, hi: f64| {
        let pad = tol.max(1e-4 * (hi - lo));
        (lo - pad, hi + pad)
    };
    // u angles (None at a pole, apex or axis), v values.
    let singular = |rho: f64, scale: f64| rho <= 1e-9 * scale.max(1.0);
    let (us, vs): (Vec<Option<f64>>, Vec<f64>) = match &*surf {
        Surf::Plane(f) => outer.iter().map(|p| f.local(*p)).map(|l| (Some(l[0]), l[1])).unzip(),
        Surf::Cylinder(f, _) => outer.iter().map(|p| f.local(*p)).map(|l| (Some(l[1].atan2(l[0])), l[2])).unzip(),
        Surf::Cone(f, r, _) => outer.iter().map(|p| f.local(*p)).map(|l| ((!singular(l[0].hypot(l[1]), *r)).then(|| l[1].atan2(l[0])), l[2])).unzip(),
        Surf::Sphere(f, r) => outer
            .iter()
            .map(|p| f.local(*p))
            .map(|l| ((!singular(l[0].hypot(l[1]), *r)).then(|| l[1].atan2(l[0])), l[2].atan2(l[0].hypot(l[1]))))
            .unzip(),
        Surf::Torus(f, big, _) => outer
            .iter()
            .map(|p| f.local(*p))
            .map(|l| ((!singular(l[0].hypot(l[1]), *big)).then(|| l[1].atan2(l[0])), l[2].atan2(l[0].hypot(l[1]) - big)))
            .unzip(),
        Surf::Nurbs(_) => return fail("unreachable surface"),
    };
    if us.is_empty() {
        return fail("a face without a boundary");
    }
    let plane = if let Surf::Plane(f) = &*surf { Some(*f) } else { None };
    if let Some(f) = plane {
        let uv: Vec<f64> = us.iter().flatten().copied().collect();
        let (Some((u0, u1)), Some((v0, v1))) = (bounds(&uv), bounds(&vs)) else { return fail("a face without a boundary") };
        let (u0, u1) = linear(u0, u1);
        let (v0, v1) = linear(v0, v1);
        *surf = Surf::Plane(Frame { o: f.at(u0, v0, 0.0), ..f });
        return Ok([0.0, 0.0, u1 - u0, v1 - v0]);
    }
    let (uu, turns) = unwrap_angles(&us);
    let Some((mut u0, mut u1)) = bounds(&uu) else { return fail("a face without a boundary") };
    let is_sphere = matches!(surf, Surf::Sphere(..));
    // A loop round a pole, or a seam with nothing else to span the turn, covers the full period.
    if (seam || (is_sphere && turns.abs() >= 1.0)) && u1 - u0 < TAU - 1e-6 {
        u1 = u0 + TAU;
    }
    let (ulo, uhi) = if u1 - u0 + 2.0 * pad_a >= TAU - 1e-6 { (u0, u0 + TAU) } else { (u0 - pad_a, u1 + pad_a) };
    u0 = ulo;
    u1 = uhi;
    let (mut v0, mut v1) = match &*surf {
        Surf::Torus(..) => {
            let (vv, _) = unwrap_angles(&vs.iter().map(|v| Some(*v)).collect::<Vec<_>>());
            let Some((lo, hi)) = bounds(&vv) else { return fail("a face without a boundary") };
            let shift = ((lo + PI) / TAU).floor() * TAU;
            let (lo, hi) = (lo - shift, hi - shift);
            if hi - lo + 2.0 * pad_a >= TAU - 1e-6 { (lo, lo + TAU) } else { (lo - pad_a, hi + pad_a) }
        }
        Surf::Sphere(..) => {
            let Some((lo, hi)) = bounds(&vs) else { return fail("a face without a boundary") };
            // A loop round a pole reaches it: counter-clockwise (turns > 0) keeps the north side.
            let (lo, hi) = if turns >= 1.0 {
                (lo, FRAC_PI_2)
            } else if turns <= -1.0 {
                (-FRAC_PI_2, hi)
            } else {
                (lo, hi)
            };
            ((lo - pad_a).max(-FRAC_PI_2), (hi + pad_a).min(FRAC_PI_2))
        }
        Surf::Cone(_, r, a) => {
            let Some((lo, hi)) = bounds(&vs) else { return fail("a face without a boundary") };
            // The padding stops at the apex: a cone has no radius below it.
            let (lo, hi) = linear(lo, hi);
            (lo.max(-r / a.tan()), hi)
        }
        _ => {
            let Some((lo, hi)) = bounds(&vs) else { return fail("a face without a boundary") };
            linear(lo, hi)
        }
    };
    if v1 <= v0 {
        v1 = v0 + 1e-6;
        v0 -= 1e-6;
    }
    let span = (u1 - u0).min(TAU);
    *surf = match &*surf {
        Surf::Cylinder(f, r) => Surf::Cylinder(f.spun(u0), *r),
        Surf::Cone(f, r, a) => Surf::Cone(f.spun(u0), *r, *a),
        Surf::Sphere(f, r) => Surf::Sphere(f.spun(u0), *r),
        Surf::Torus(f, a, b) => Surf::Torus(f.spun(u0), *a, *b),
        other => other.clone(),
    };
    Ok([0.0, v0, span, v1])
}

/// A loop of a sphere or torus face in the surface's parameters: angles made continuous, the net
/// turns round each period, and the signed area (positive counter-clockwise) when it turns
/// round neither.
struct LoopUv {
    u: Vec<f64>,
    v: Vec<f64>,
    turns_u: f64,
    turns_v: f64,
    area: f64,
}

fn loop_uv(m: &Model, surf: &Surf, l: &[(usize, bool)]) -> Option<LoopUv> {
    let mut pts = Vec::new();
    for (ei, fwd) in l {
        pts.extend(sample_edge(m.edges.get(*ei)?, *fwd)?);
    }
    // (u, v) of each point; u is missing at a pole.
    let singular = |rho: f64, scale: f64| rho <= 1e-9 * scale.max(1.0);
    let uv: Vec<(Option<f64>, f64)> = pts
        .iter()
        .map(|p| match surf {
            Surf::Sphere(f, r) => {
                let q = f.local(*p);
                let rho = q[0].hypot(q[1]);
                Some(((!singular(rho, *r)).then(|| q[1].atan2(q[0])), q[2].atan2(rho)))
            }
            Surf::Torus(f, big, _) => {
                let q = f.local(*p);
                let rho = q[0].hypot(q[1]);
                Some(((!singular(rho, *big)).then(|| q[1].atan2(q[0])), q[2].atan2(rho - big)))
            }
            _ => None,
        })
        .collect::<Option<_>>()?;
    let kept: Vec<(f64, f64)> = uv.iter().filter_map(|(u, v)| u.map(|u| (u, *v))).collect();
    let (u, turns_u) = unwrap_angles(&kept.iter().map(|k| Some(k.0)).collect::<Vec<_>>());
    let (v, turns_v) = if matches!(surf, Surf::Torus(..)) {
        unwrap_angles(&kept.iter().map(|k| Some(k.1)).collect::<Vec<_>>())
    } else {
        (kept.iter().map(|k| k.1).collect(), 0.0)
    };
    let pairs: Vec<(f64, f64)> = u.iter().copied().zip(v.iter().copied()).collect();
    let area = pairs.iter().zip(pairs.iter().cycle().skip(1)).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum::<f64>() / 2.0;
    Some(LoopUv { u, v, turns_u, turns_v, area })
}

/// Make the arc's ends exactly the given points.
fn pin_ends(c: &mut Nurbs, a: V3, b: V3) {
    if let Some(q) = c.pts.first_mut() {
        *q = a;
    }
    if let Some(q) = c.pts.last_mut() {
        *q = b;
    }
}

/// A face on a sphere or torus whose boundary does not close in the parameter box cannot be
/// written as it is: another reader finds no region inside a loop that is the outside of a hole
/// or that winds round a pole. So close it the way a whole sphere or torus is closed, with seams
/// that run along a meridian (and a parallel) up to the poles, and keep the old loop as a hole:
///
/// - a loop turning once round a pole gets a meridian from one of its vertices to that pole,
///   used both ways;
/// - a loop that is a clockwise hole on a sphere gets a meridian from pole to pole, used both ways,
///   as its outer loop (set in the gap beside the hole);
/// - a clockwise hole on a torus gets a meridian circle and a parallel circle through one new vertex
///   (set in the gaps beside the hole), run round the parameter box as the outer loop.
///
/// Other faces are left as they are.
fn close_periodic_face(m: &mut Model, surf: &Surf, loops: &mut Vec<Vec<(usize, bool)>>) -> Option<()> {
    // A loop that already runs an edge both ways has its seam.
    let outer = loops.first()?;
    if outer.iter().any(|(e, fwd)| outer.contains(&(*e, !*fwd))) {
        return None;
    }
    let lu = loop_uv(m, surf, outer)?;
    let gap = |v: &[f64]| bounds(v).filter(|(a, b)| b - a < TAU - 0.2).map(|(a, b)| b + (TAU - (b - a)) / 2.0);
    match surf {
        Surf::Sphere(f, r) if lu.turns_u.abs() >= 1.0 => {
            let (e0, fwd0) = *loops.first()?.first()?;
            let ed = m.edges.get(e0)?;
            let v0 = if fwd0 { ed.v.0 } else { ed.v.1 };
            let p0 = *m.verts.get(v0)?;
            let axis = if lu.turns_u > 0.0 { f.z } else { mul(f.z, -1.0) };
            let pole = add(f.o, mul(axis, *r));
            let xf = unit(sub(p0, f.o))?;
            let yf = unit(sub(axis, mul(xf, dot(axis, xf))))?;
            let frame = Frame::new(f.o, cross(xf, yf), xf)?;
            let mut c = Nurbs::arc(&frame, *r, *r, 0.0, dot(axis, yf).atan2(dot(axis, xf)));
            pin_ends(&mut c, p0, pole);
            m.verts.push(pole);
            let range = c.domain()?;
            m.edges.push(EdgeM { curve: c, range, v: (v0, m.verts.len() - 1) });
            let seam = m.edges.len() - 1;
            loops.first_mut()?.extend([(seam, true), (seam, false)]);
        }
        Surf::Sphere(f, r) if lu.turns_u == 0.0 && lu.area < -1e-9 => {
            let u0 = gap(&lu.u)?;
            let meridian = add(mul(f.x, u0.cos()), mul(f.y, u0.sin()));
            let (south, north) = (add(f.o, mul(f.z, -*r)), add(f.o, mul(f.z, *r)));
            let xf = mul(f.z, -1.0);
            let frame = Frame::new(f.o, cross(xf, meridian), xf)?;
            let mut c = Nurbs::arc(&frame, *r, *r, 0.0, PI);
            pin_ends(&mut c, south, north);
            m.verts.extend([south, north]);
            let range = c.domain()?;
            let nv = m.verts.len();
            m.edges.push(EdgeM { curve: c, range, v: (nv - 2, nv - 1) });
            let seam = m.edges.len() - 1;
            loops.insert(0, vec![(seam, true), (seam, false)]);
        }
        Surf::Torus(f, big, small) if lu.turns_u == 0.0 && lu.turns_v == 0.0 && lu.area < -1e-9 => {
            let (u0, v0) = (gap(&lu.u)?, gap(&lu.v)?);
            let radial = add(mul(f.x, u0.cos()), mul(f.y, u0.sin()));
            let rho = big + small * v0.cos();
            let corner = add(f.o, add(mul(radial, rho), mul(f.z, small * v0.sin())));
            // The tube circle at u0 (v runs round it) and the circle of latitude v0 (u runs round it).
            let tube = Frame::new(add(f.o, mul(radial, *big)), cross(radial, f.z), radial)?;
            let parallel = Frame::new(add(f.o, mul(f.z, small * v0.sin())), f.z, f.x)?;
            let (mut c0, mut c1) = (Nurbs::arc(&tube, *small, *small, v0, TAU), Nurbs::arc(&parallel, rho, rho, u0, TAU));
            pin_ends(&mut c0, corner, corner);
            pin_ends(&mut c1, corner, corner);
            m.verts.push(corner);
            let q = m.verts.len() - 1;
            for c in [c0, c1] {
                let range = c.domain()?;
                m.edges.push(EdgeM { curve: c, range, v: (q, q) });
            }
            let (e0, e1) = (m.edges.len() - 2, m.edges.len() - 1);
            loops.insert(0, vec![(e0, false), (e1, true), (e0, true), (e1, false)]);
        }
        _ => {}
    }
    Some(())
}

/// The outer-first, normal-side loops of a STEP face: (surface, face sense, loops of (EDGE_CURVE, runs along it)).
fn face_data(s: &S, fid: u64) -> Result<(u64, bool, Vec<Vec<(u64, bool)>>)> {
    let fe = s.e(fid)?;
    if !matches!(fe.name(), "ADVANCED_FACE" | "FACE_SURFACE") {
        return fail(format!("not supported yet: face {}", fe.name()));
    }
    let sense = fe.params().get(3).and_then(Param::as_bool).unwrap_or(true);
    let surf = s.r(fe.params().get(2))?;
    let all = s.list(fe.params().get(1))?;
    let is_outer = |b: &Param| b.as_ref_id().and_then(|x| s.ex.get(x)).is_some_and(|e| e.name() == "FACE_OUTER_BOUND");
    let mut ordered: Vec<&Param> = all.iter().filter(|b| is_outer(b)).collect();
    ordered.extend(all.iter().filter(|b| !is_outer(b)));
    let mut loops = Vec::new();
    for b in ordered {
        let be = s.e(s.r(Some(b))?)?;
        let orient = be.params().get(2).and_then(Param::as_bool).unwrap_or(true);
        let lp = s.e(s.r(be.params().get(1))?)?;
        if lp.name() != "EDGE_LOOP" {
            return fail(format!("not supported yet: face bound {}", lp.name()));
        }
        let mut l = Vec::new();
        for oe in s.list(lp.params().get(1))? {
            let oe = s.e(s.r(Some(oe))?)?;
            if oe.name() != "ORIENTED_EDGE" {
                return fail(format!("not supported yet: loop member {}", oe.name()));
            }
            l.push((s.r(oe.params().get(3))?, oe.params().get(4).and_then(Param::as_bool).unwrap_or(true)));
        }
        // Stored as seen from the normal side: reverse when the bound and the face disagree.
        if orient != sense {
            l.reverse();
            for x in &mut l {
                x.1 = !x.1;
            }
        }
        if l.is_empty() {
            return fail("an empty loop");
        }
        loops.push(l);
    }
    if loops.is_empty() {
        return fail("a face without a boundary");
    }
    Ok((surf, sense, loops))
}

/// Restate the shells `(shell, material side reversed)` as a model.
fn build_model(s: &S, shells: &[(u64, bool)], sheet: bool, tol: f64) -> Result<Model> {
    let mut m = Model { verts: Vec::new(), edges: Vec::new(), faces: Vec::new(), shells: shells.len(), sheet, gap: 0.0 };
    let (mut vmap, mut emap): (HashMap<u64, usize>, HashMap<u64, usize>) = (HashMap::new(), HashMap::new());
    for (si, (shell, flip)) in shells.iter().enumerate() {
        let sh = s.e(*shell)?;
        for f in s.list(sh.params().get(1))? {
            if m.faces.len() >= MAX_ITEMS {
                return fail("too many faces");
            }
            let (surf_id, sense, step_loops) = face_data(s, s.r(Some(f))?)?;
            let mut surf = surface(s, surf_id)?;
            let mut loops = Vec::new();
            for sl in step_loops {
                let mut l = Vec::new();
                for (ec, forward) in sl {
                    let ei = match emap.get(&ec) {
                        Some(i) => *i,
                        None => {
                            let ee = s.e(ec)?;
                            let p = ee.params();
                            let (v0, v1) = (s.r(p.get(1))?, s.r(p.get(2))?);
                            let (pa, pb) = (s.vertex(v0)?, s.vertex(v1)?);
                            let same = p.get(4).and_then(Param::as_bool).unwrap_or(true);
                            let g = edge_geometry(s, s.r(p.get(3))?, same, v0 == v1, pa, pb, tol)?;
                            let mut vi = |id: u64, at: V3| {
                                *vmap.entry(id).or_insert_with(|| {
                                    m.verts.push(at);
                                    m.verts.len() - 1
                                })
                            };
                            let (a, b) = (vi(v0, pa), vi(v1, pb));
                            m.edges.push(EdgeM { curve: g.curve, range: g.range, v: (a, b) });
                            emap.insert(ec, m.edges.len() - 1);
                            m.edges.len() - 1
                        }
                    };
                    l.push((ei, forward));
                }
                // Head to tail.
                for (k, (ei, fwd)) in l.iter().enumerate() {
                    let (nei, nfwd) = l.get((k + 1) % l.len()).copied().unwrap_or((*ei, *fwd));
                    let end = m.edges.get(*ei).map(|e| if *fwd { e.v.1 } else { e.v.0 });
                    let start = m.edges.get(nei).map(|e| if nfwd { e.v.0 } else { e.v.1 });
                    if end.is_none() || end != start {
                        return fail("a loop whose edges do not meet end to start");
                    }
                }
                loops.push(l);
            }
            if matches!(surf, Surf::Sphere(..) | Surf::Torus(..)) {
                let _ = close_periodic_face(&mut m, &surf, &mut loops);
            }
            // Samples of the outer loop for the parameter box, and the gaps to the surface.
            let mut outer = Vec::new();
            let mut seam = false;
            for (k, l) in loops.iter().enumerate() {
                for (ei, fwd) in l {
                    let Some(e) = m.edges.get(*ei) else { return fail("a missing edge") };
                    let Some(pts) = sample_edge(e, *fwd) else { return fail("an edge that cannot be evaluated") };
                    for p in &pts {
                        m.gap = m.gap.max(surface_gap(&surf, *p));
                    }
                    if k == 0 {
                        outer.extend(pts);
                        seam |= l.iter().any(|(oi, of)| oi == ei && *of != *fwd);
                    }
                }
            }
            let range = fit_range(&mut surf, &outer, seam, tol)?;
            m.faces.push(Face { surf, range, loops, outward_natural: sense != *flip, shell: si });
        }
    }
    if m.faces.is_empty() {
        return fail("a shell without faces");
    }
    Ok(m)
}

// ---------------------------------------------------------------------------------------------
// The flat arrays

/// An analytic surface type's arrays: origin (centre), axis, reference direction, scalars.
#[derive(Default)]
struct Ana {
    o: Vec<V3>,
    z: Vec<V3>,
    x: Vec<V3>,
    k1: Vec<f64>,
    k2: Vec<f64>,
}

impl Ana {
    fn push(&mut self, f: &Frame, k1: Option<f64>, k2: Option<f64>) {
        self.o.push(f.o);
        self.z.push(f.z);
        self.x.push(f.x);
        self.k1.extend(k1);
        self.k2.extend(k2);
    }
}

#[derive(Default)]
struct Arrays {
    tol: Vec<f64>,
    extent: Vec<V3>,
    regions: Vec<u32>,
    region_shells: Vec<u32>,
    region_type: Vec<&'static str>,
    shell_fu: Vec<u32>,
    fu_face: Vec<u32>,
    fu_orient: Vec<&'static str>,
    face_loops: Vec<u32>,
    face_type: Vec<&'static str>,
    face_range: Vec<[f64; 2]>,
    loop_eu: Vec<u32>,
    eu_edge: Vec<u32>,
    eu_orient: Vec<&'static str>,
    eu_next: Vec<u32>,
    eu_entry: Vec<&'static str>,
    edge_range: Vec<f64>,
    edge_vi: Vec<[u32; 2]>,
    verts: Vec<V3>,
    cv: Vec<V3>,
    cv_n: Vec<u32>,
    cv_order: Vec<u32>,
    cv_knots: Vec<f64>,
    cv_w: Vec<f64>,
    s_cv: Vec<V3>,
    s_nu: Vec<u32>,
    s_nv: Vec<u32>,
    s_ou: Vec<u32>,
    s_ov: Vec<u32>,
    s_ku: Vec<f64>,
    s_kv: Vec<f64>,
    s_w: Vec<f64>,
    plane: Ana,
    cyl: Ana,
    cone: Ana,
    sphere: Ana,
    torus: Ana,
}

fn u32_of(x: usize) -> Result<u32> {
    match u32::try_from(x) {
        Ok(v) if x < MAX_ITEMS => Ok(v),
        _ => fail("too many items"),
    }
}

impl Arrays {
    /// Every number to be written is finite (checked here, not in the text, where a body's name
    /// may spell "NaN" or "inf").
    fn finite(&self) -> bool {
        let ana = |a: &Ana| [&a.o, &a.z, &a.x].iter().all(|v| v.iter().all(|p| finite3(*p))) && a.k1.iter().chain(&a.k2).all(|x| x.is_finite());
        [&self.tol, &self.edge_range, &self.cv_knots, &self.cv_w, &self.s_ku, &self.s_kv, &self.s_w].iter().all(|v| v.iter().all(|x| x.is_finite()))
            && [&self.extent, &self.verts, &self.cv, &self.s_cv].iter().all(|v| v.iter().all(|p| finite3(*p)))
            && self.face_range.iter().all(|r| r.iter().all(|x| x.is_finite()))
            && [&self.plane, &self.cyl, &self.cone, &self.sphere, &self.torus].into_iter().all(ana)
    }
}

const SAME: &str = "same";
const OPPOSITE: &str = "opposite";

/// Add one brep to the arrays (indices become global).
fn append(a: &mut Arrays, m: &Model) -> Result<()> {
    let (v_off, e_off, f_off) = (a.verts.len(), a.edge_vi.len(), a.face_loops.len());
    let mut points: Vec<V3> = m.verts.clone();
    a.verts.extend(&m.verts);
    for e in &m.edges {
        a.edge_range.extend([e.range.0, e.range.1]);
        a.edge_vi.push([u32_of(e.v.0 + v_off)?, u32_of(e.v.1 + v_off)?]);
        a.cv_n.push(u32_of(e.curve.pts.len())?);
        a.cv_order.push(u32_of(e.curve.order)?);
        a.cv.extend(&e.curve.pts);
        a.cv_knots.extend(&e.curve.knots);
        a.cv_w.extend(&e.curve.w);
        points.extend(&e.curve.pts);
    }
    // Faces: geometry, loops, edgeuses.
    let mut uses: Vec<Vec<usize>> = vec![Vec::new(); m.edges.len()];
    for f in &m.faces {
        let face_type = match &f.surf {
            Surf::Nurbs(n) => {
                a.s_cv.extend(&n.pts);
                a.s_nu.push(u32_of(n.nu)?);
                a.s_nv.push(u32_of(n.nv)?);
                a.s_ou.push(u32_of(n.ou)?);
                a.s_ov.push(u32_of(n.ov)?);
                a.s_ku.extend(&n.ku);
                a.s_kv.extend(&n.kv);
                a.s_w.extend(&n.w);
                points.extend(&n.pts);
                "BrepSurfaceNurbAPI"
            }
            Surf::Plane(fr) => {
                a.plane.push(fr, None, None);
                "BrepSurfacePlaneAPI"
            }
            Surf::Cylinder(fr, r) => {
                a.cyl.push(fr, Some(*r), None);
                "BrepSurfaceCylinderAPI"
            }
            Surf::Cone(fr, r, ang) => {
                a.cone.push(fr, Some(*r), Some(*ang));
                "BrepSurfaceConeAPI"
            }
            Surf::Sphere(fr, r) => {
                a.sphere.push(fr, Some(*r), None);
                "BrepSurfaceSphereAPI"
            }
            Surf::Torus(fr, big, small) => {
                a.torus.push(fr, Some(*big), Some(*small));
                "BrepSurfaceTorusAPI"
            }
        };
        a.face_type.push(face_type);
        a.face_range.push([f.range[0], f.range[1]]);
        a.face_range.push([f.range[2], f.range[3]]);
        a.face_loops.push(u32_of(f.loops.len())?);
        for l in &f.loops {
            a.loop_eu.push(u32_of(l.len())?);
            for (ei, forward) in l {
                let idx = a.eu_edge.len();
                a.eu_edge.push(u32_of(ei + e_off)?);
                a.eu_orient.push(if *forward { SAME } else { OPPOSITE });
                // Face on the left of the loop: along the edge enters from the bottom.
                a.eu_entry.push(if *forward { "bottomEntry" } else { "topEntry" });
                a.eu_next.push(u32_of(idx)?);
                match uses.get_mut(*ei) {
                    Some(u) => u.push(idx),
                    None => return fail("a missing edge"),
                }
            }
        }
    }
    // Radial cycles.
    for us in &uses {
        match us.as_slice() {
            [x] if m.sheet => {
                if let Some(n) = a.eu_next.get_mut(*x) {
                    *n = u32_of(*x)?;
                }
            }
            [x, y] => {
                if let Some(n) = a.eu_next.get_mut(*x) {
                    *n = u32_of(*y)?;
                }
                if let Some(n) = a.eu_next.get_mut(*y) {
                    *n = u32_of(*x)?;
                }
            }
            other => return fail(format!("not supported yet: an edge used by {} faces", other.len())),
        }
    }
    // Regions, shells, faceuses.
    let in_shell = |si: usize| m.faces.iter().enumerate().filter(move |(_, f)| f.shell == si);
    let face_ix = |i: usize| u32_of(i + f_off);
    let region = |a: &mut Arrays, ty: &'static str, shells: &[Vec<(u32, &'static str)>]| -> Result<()> {
        a.region_shells.push(u32_of(shells.len())?);
        a.region_type.push(ty);
        for sh in shells {
            a.shell_fu.push(u32_of(sh.len())?);
            for (f, o) in sh {
                a.fu_face.push(*f);
                a.fu_orient.push(o);
            }
        }
        Ok(())
    };
    let side = |si: usize, outward: bool| -> Result<Vec<(u32, &'static str)>> {
        in_shell(si).map(|(i, f)| Ok((face_ix(i)?, if f.outward_natural == outward { SAME } else { OPPOSITE }))).collect()
    };
    if m.sheet {
        let shells = (0..m.shells)
            .map(|si| {
                let mut v = Vec::new();
                for (i, _) in in_shell(si) {
                    v.push((face_ix(i)?, SAME));
                    v.push((face_ix(i)?, OPPOSITE));
                }
                Ok(v)
            })
            .collect::<Result<Vec<_>>>()?;
        region(a, "voidRegion", &shells)?;
        a.regions.push(1);
    } else {
        region(a, "voidRegion", &[side(0, true)?])?;
        let mut solid = vec![side(0, false)?];
        for si in 1..m.shells {
            solid.push(side(si, false)?);
        }
        region(a, "solidRegion", &solid)?;
        for si in 1..m.shells {
            region(a, "voidRegion", &[side(si, true)?])?;
        }
        a.regions.push(u32_of(m.shells + 1)?);
    }
    // Tolerance and extent: every vertex and control point inside.
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if !(finite3(lo) && finite3(hi)) {
        return fail("a brep without points");
    }
    let tol = (norm(sub(hi, lo)) * 1e-6).max(1e-5).max(2.0 * m.gap);
    a.tol.push(tol);
    a.extent.push(sub(lo, [tol; 3]));
    a.extent.push(add(hi, [tol; 3]));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The text

fn f(x: f64) -> String {
    if x == 0.0 { "0".into() } else { format!("{x:?}") }
}

fn v3s(v: &[V3]) -> String {
    let mut s = String::from("[");
    for (i, p) in v.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let _ = write!(s, "({}, {}, {})", f(p[0]), f(p[1]), f(p[2]));
    }
    s.push(']');
    s
}

fn list<T>(v: &[T], item: impl Fn(&T) -> String) -> String {
    format!("[{}]", v.iter().map(item).collect::<Vec<_>>().join(", "))
}

fn nums(v: &[f64]) -> String {
    list(v, |x| f(*x))
}

fn uints(v: &[u32]) -> String {
    list(v, |x| x.to_string())
}

fn toks(v: &[&str]) -> String {
    list(v, |x| format!("\"{x}\""))
}

fn f32_down(x: f64) -> f32 {
    let y = x as f32;
    if f64::from(y) > x { y.next_down() } else { y }
}

fn f32_up(x: f64) -> f32 {
    let y = x as f32;
    if f64::from(y) < x { y.next_up() } else { y }
}

/// A USD identifier for a body name: `[A-Za-z_][A-Za-z0-9_]*`, never one already `used`
/// (a repeat gets `_2`, `_3`, ...).
fn identifier(name: &str, used: &mut HashSet<String>) -> String {
    let mut base: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if base.is_empty() {
        base = "Body".into();
    } else if base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert(0, '_');
    }
    let mut id = base.clone();
    let mut n = 1;
    while !used.insert(id.clone()) {
        n += 1;
        id = format!("{base}_{n}");
    }
    id
}

/// A USD string literal (quotes and backslashes escaped, control characters made spaces).
fn quoted(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if c.is_control() => o.push(' '),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// One `def BrepArray` prim for the brep in `a`; `label` is the body's own name.
fn render(a: &Arrays, ident: &str, label: &str, color: Option<[f32; 3]>) -> Result<String> {
    if !a.finite() {
        return fail("a number is not finite");
    }
    let mut apis = vec!["BrepPointAPI:vertexPoint".to_string(), "BrepCurve3dNurbAPI:edge3dNurb".to_string()];
    for (used, api) in [
        (!a.s_nu.is_empty(), "BrepSurfaceNurbAPI"),
        (!a.plane.o.is_empty(), "BrepSurfacePlaneAPI"),
        (!a.cyl.o.is_empty(), "BrepSurfaceCylinderAPI"),
        (!a.cone.o.is_empty(), "BrepSurfaceConeAPI"),
        (!a.sphere.o.is_empty(), "BrepSurfaceSphereAPI"),
        (!a.torus.o.is_empty(), "BrepSurfaceTorusAPI"),
    ] {
        if used {
            apis.push(api.to_string());
        }
    }
    let api_list = list(&apis, |x| format!("\"{x}\""));
    // The prim's own extent: every brep extent, rounded outwards to single precision.
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &a.extent {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let ext = format!(
        "[({:?}, {:?}, {:?}), ({:?}, {:?}, {:?})]",
        f32_down(lo[0]),
        f32_down(lo[1]),
        f32_down(lo[2]),
        f32_up(hi[0]),
        f32_up(hi[1]),
        f32_up(hi[2])
    );
    let mut o = String::new();
    let _ = write!(
        o,
        "    def BrepArray \"{ident}\" (\n        prepend apiSchemas = {api_list}\n        customData = {{\n            dictionary solidcraft = {{\n                string name = {}\n            }}\n        }}\n    )\n    {{\n",
        quoted(label)
    );
    let mut put = |ty: &str, name: &str, val: String| {
        let _ = writeln!(o, "        uniform {ty} {name} = {val}");
    };
    let loop_vertex = vec![NO_VERTEX; a.loop_eu.len()];
    let shell_zero = vec![0u32; a.shell_fu.len()];
    let shell_none = vec!["none"; a.shell_fu.len()];
    let vertex_type = vec!["BrepPointAPI"; a.verts.len()];
    let edge_type = vec!["BrepCurve3dNurbAPI"; a.edge_vi.len()];
    let edge_vi: Vec<String> = a.edge_vi.iter().map(|v| format!("({}, {})", v[0], v[1])).collect();
    let face_range: Vec<String> = a.face_range.iter().map(|r| format!("({}, {})", f(r[0]), f(r[1]))).collect();
    let extent: Vec<String> = a.extent.iter().map(|p| format!("({}, {}, {})", f(p[0]), f(p[1]), f(p[2]))).collect();
    put("point3d[]", "brep:edge3dNurb:curve3d:nurb:controlVertices", v3s(&a.cv));
    put("double[]", "brep:edge3dNurb:curve3d:nurb:knots", nums(&a.cv_knots));
    put("uint[]", "brep:edge3dNurb:curve3d:nurb:order", uints(&a.cv_order));
    put("uint[]", "brep:edge3dNurb:curve3d:nurb:vertexCount", uints(&a.cv_n));
    put("double[]", "brep:edge3dNurb:curve3d:nurb:weights", nums(&a.cv_w));
    put("double3[]", "brep:extent", format!("[{}]", extent.join(", ")));
    put("double[]", "brep:intersectTol3d", nums(&a.tol));
    put("uint[]", "brep:regionCount", uints(&a.regions));
    if !a.s_nu.is_empty() {
        put("point3d[]", "brep:surface:nurb:controlVertices", v3s(&a.s_cv));
        put("double[]", "brep:surface:nurb:uKnots", nums(&a.s_ku));
        put("uint[]", "brep:surface:nurb:uOrder", uints(&a.s_ou));
        put("uint[]", "brep:surface:nurb:uVertexCount", uints(&a.s_nu));
        put("double[]", "brep:surface:nurb:vKnots", nums(&a.s_kv));
        put("uint[]", "brep:surface:nurb:vOrder", uints(&a.s_ov));
        put("uint[]", "brep:surface:nurb:vVertexCount", uints(&a.s_nv));
        put("double[]", "brep:surface:nurb:weights", nums(&a.s_w));
    }
    for (prefix, o_name, ana, scalars) in [
        ("plane", "origin", &a.plane, &[][..]),
        ("cylinder", "origin", &a.cyl, &["radius"][..]),
        ("cone", "origin", &a.cone, &["radius", "semiAngle"][..]),
        ("sphere", "center", &a.sphere, &["radius"][..]),
        ("torus", "origin", &a.torus, &["majorRadius", "minorRadius"][..]),
    ] {
        if ana.o.is_empty() {
            continue;
        }
        put("vector3d[]", &format!("brep:surface:{prefix}:axis"), v3s(&ana.z));
        put("point3d[]", &format!("brep:surface:{prefix}:{o_name}"), v3s(&ana.o));
        put("vector3d[]", &format!("brep:surface:{prefix}:refDirection"), v3s(&ana.x));
        for (name, vals) in scalars.iter().zip([&ana.k1, &ana.k2]) {
            put("double[]", &format!("brep:surface:{prefix}:{name}"), nums(vals));
        }
    }
    put("point3d[]", "brep:vertexPoint:point:position", v3s(&a.verts));
    put("token[]", "edge:curveType", toks(&edge_type));
    put("double[]", "edge:range", nums(&a.edge_range));
    put("int2[]", "edge:vertexIndices", format!("[{}]", edge_vi.join(", ")));
    put("uint[]", "edgeuse:edgeIndex", uints(&a.eu_edge));
    put("uint[]", "edgeuse:nextRadialEUIndex", uints(&a.eu_next));
    put("token[]", "edgeuse:orientationType", toks(&a.eu_orient));
    put("token[]", "edgeuse:thisRadialEntryType", toks(&a.eu_entry));
    put("float3[]", "extent", ext);
    put("uint[]", "face:loopCount", uints(&a.face_loops));
    put("double2[]", "face:range", format!("[{}]", face_range.join(", ")));
    put("token[]", "face:surfaceType", toks(&a.face_type));
    put("token[]", "face:trimType", toks(&vec!["general"; a.face_type.len()]));
    put("uint[]", "faceuse:faceIndex", uints(&a.fu_face));
    put("token[]", "faceuse:orientationType", toks(&a.fu_orient));
    put("uint[]", "loop:edgeuseCount", uints(&a.loop_eu));
    put("uint[]", "loop:vertexIndex", uints(&loop_vertex));
    put("uint[]", "region:shellCount", uints(&a.region_shells));
    put("token[]", "region:type", toks(&a.region_type));
    put("uint[]", "shell:faceuseCount", uints(&a.shell_fu));
    put("token[]", "shell:pointType", toks(&shell_none));
    put("uint[]", "shell:wireEdgeCount", uints(&shell_zero));
    put("token[]", "vertex:pointType", toks(&vertex_type));
    put("token[]", "wireEdge:curveType", "[]".into());
    put("double[]", "wireEdge:range", "[]".into());
    put("int2[]", "wireEdge:vertexIndices", "[]".into());
    if let Some(c) = color {
        let c = c.map(|x| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 });
        let _ = writeln!(o, "        color3f[] primvars:displayColor = [({:?}, {:?}, {:?})]", c[0], c[1], c[2]);
        o.push_str("        uniform token primvars:displayColor:interpolation = \"constant\"\n");
    }
    o.push_str("    }\n");
    Ok(o)
}

// ---------------------------------------------------------------------------------------------
// Entry point

/// A shell reference: the shell and whether its orientation is reversed.
fn shell_ref(s: &S, id: u64) -> Result<(u64, bool)> {
    let e = s.e(id)?;
    match e.name() {
        "ORIENTED_CLOSED_SHELL" | "ORIENTED_OPEN_SHELL" => Ok((s.r(e.params().get(2))?, !e.params().get(3).and_then(Param::as_bool).unwrap_or(true))),
        _ => Ok((id, false)),
    }
}

/// The breps of one body's STEP export: one for each solid (a body in several pieces writes
/// several) or sheet, packed in one set of arrays.
fn body_arrays(body: &ExportBody, header: &StepHeader) -> Result<Arrays> {
    let step = crate::step_export_bodies(std::slice::from_ref(body), header)?;
    let ex = p21::parse(&step).map_err(|e| KernelError::Failed(format!("USD export: {e}")))?;
    let s = S { ex: &ex };
    let mut max_coord: f64 = 1.0;
    for p in ex.entities.values().filter(|e| e.name() == "CARTESIAN_POINT") {
        if let Some(c) = p.params().get(1).and_then(Param::as_list) {
            for x in c.iter().filter_map(Param::as_f64) {
                max_coord = max_coord.max(x.abs());
            }
        }
    }
    let tol = (max_coord * 1e-6).max(1e-6);
    let mut items: Vec<u64> = ex
        .entities
        .iter()
        .filter(|(_, e)| matches!(e.name(), "MANIFOLD_SOLID_BREP" | "BREP_WITH_VOIDS" | "SHELL_BASED_SURFACE_MODEL"))
        .map(|(i, _)| *i)
        .collect();
    items.sort_unstable();
    let mut arrays = Arrays::default();
    for id in items {
        let e = s.e(id)?;
        let (shells, sheet) = match e.name() {
            "SHELL_BASED_SURFACE_MODEL" => {
                (s.list(e.params().get(1))?.iter().map(|r| s.r(Some(r)).and_then(|id| shell_ref(&s, id))).collect::<Result<Vec<_>>>()?, true)
            }
            name => {
                let mut v = vec![shell_ref(&s, s.r(e.params().get(1))?)?];
                if name == "BREP_WITH_VOIDS" {
                    for r in s.list(e.params().get(2))? {
                        v.push(shell_ref(&s, s.r(Some(r))?)?);
                    }
                }
                (v, false)
            }
        };
        append(&mut arrays, &build_model(&s, &shells, sheet, tol)?)?;
    }
    Ok(arrays)
}

/// USD text (`.usda`, millimetres, Z up) with one `BrepArray` prim for each body.
///
/// Each body is exported to STEP on its own, so a prim always holds the solids of the body it is
/// named after and coloured for (a body in several pieces gives one prim with several breps; a
/// body that writes no solid gives no prim).
pub fn usd_export_bodies(bodies: &[ExportBody], header: &StepHeader) -> Result<String> {
    let mut used = HashSet::new();
    let mut out =
        String::from("#usda 1.0\n(\n    defaultPrim = \"World\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n\ndef Xform \"World\"\n{\n");
    for body in bodies {
        let arrays = body_arrays(body, header)?;
        if !arrays.regions.is_empty() {
            out.push_str(&render(&arrays, &identifier(&body.name, &mut used), &body.name, body.color)?);
        }
    }
    if used.is_empty() {
        return Err(KernelError::Invalid("nothing to export".into()));
    }
    out.push_str("}\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Body, BoolOp, boolean, box_solid, cylinder, fillet, sphere, torus};
    use solidcraft_geom::Vec3;

    /// The numbers (flattened) and tokens of each `uniform` attribute of the prim.
    #[derive(Default)]
    struct Doc {
        nums: HashMap<String, Vec<f64>>,
        toks: HashMap<String, Vec<String>>,
    }

    fn read(text: &str) -> Doc {
        let mut d = Doc::default();
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("uniform ") else { continue };
            let (head, val) = rest.split_once(" = ").unwrap();
            let name = head.split_whitespace().nth(1).unwrap().to_string();
            if val.contains('"') {
                d.toks.insert(name, val.split('"').skip(1).step_by(2).map(str::to_string).collect());
            } else {
                let flat = val.replace(['(', ')', '[', ']', ','], " ");
                d.nums.insert(name, flat.split_whitespace().map(|x| x.parse::<f64>().unwrap()).collect());
            }
        }
        d
    }

    impl Doc {
        fn f(&self, n: &str) -> &Vec<f64> {
            self.nums.get(n).unwrap_or_else(|| panic!("missing {n}"))
        }
        fn u(&self, n: &str) -> Vec<usize> {
            self.f(n).iter().map(|x| *x as usize).collect()
        }
        fn v3(&self, n: &str) -> Vec<V3> {
            self.f(n).chunks(3).map(|c| [c[0], c[1], c[2]]).collect()
        }
        fn t(&self, n: &str) -> &Vec<String> {
            self.toks.get(n).unwrap_or_else(|| panic!("missing {n}"))
        }
        fn has(&self, n: &str) -> bool {
            self.nums.contains_key(n) || self.toks.contains_key(n)
        }
    }

    fn export(bodies: &[(&str, &Body)]) -> String {
        let eb: Vec<ExportBody> = bodies.iter().map(|(n, b)| ExportBody { name: (*n).into(), body: b, color: None }).collect();
        usd_export_bodies(&eb, &StepHeader { file_name: "t.usda".into(), ..Default::default() }).unwrap()
    }

    fn sum(v: &[usize]) -> usize {
        v.iter().sum()
    }

    /// Offsets of consecutive groups of the given sizes.
    fn starts(sizes: &[usize]) -> Vec<usize> {
        let mut o = vec![0];
        for s in sizes {
            o.push(o.last().unwrap() + s);
        }
        o
    }

    /// The NURBS curve and parameter range of every edge.
    fn edge_curves(d: &Doc) -> Vec<(Nurbs, (f64, f64))> {
        let cv_n = d.u("brep:edge3dNurb:curve3d:nurb:vertexCount");
        let cv_order = d.u("brep:edge3dNurb:curve3d:nurb:order");
        let cv = d.v3("brep:edge3dNurb:curve3d:nurb:controlVertices");
        let w = d.f("brep:edge3dNurb:curve3d:nurb:weights");
        let kn = d.f("brep:edge3dNurb:curve3d:nurb:knots");
        let cv_starts = starts(&cv_n);
        let kn_starts = starts(&cv_n.iter().zip(&cv_order).map(|(n, o)| n + o).collect::<Vec<_>>());
        let er = d.f("edge:range");
        (0..cv_n.len())
            .map(|e| {
                let c = Nurbs {
                    order: cv_order[e],
                    pts: cv[cv_starts[e]..cv_starts[e + 1]].to_vec(),
                    w: w[cv_starts[e]..cv_starts[e + 1]].to_vec(),
                    knots: kn[kn_starts[e]..kn_starts[e + 1]].to_vec(),
                };
                (c, (er[2 * e], er[2 * e + 1]))
            })
            .collect()
    }

    /// The text of each `def BrepArray` prim.
    fn prims(text: &str) -> Vec<String> {
        text.split("\n    def BrepArray ").skip(1).map(|p| format!("    def BrepArray {p}")).collect()
    }

    /// The stage metadata, and every prim checked on its own.
    fn check_all(text: &str) -> Vec<Doc> {
        assert!(text.starts_with("#usda 1.0\n"));
        assert!(text.contains("upAxis = \"Z\"") && text.contains("metersPerUnit = 0.001") && text.contains("defaultPrim = \"World\""));
        assert!(text.contains("def Xform \"World\"\n{\n") && text.ends_with("}\n"));
        let ps = prims(text);
        assert_eq!(ps.len(), text.matches("def BrepArray").count());
        ps.iter().map(|p| check_prim(p)).collect()
    }

    /// A text with one prim, checked.
    fn check(text: &str) -> Doc {
        let mut ds = check_all(text);
        assert_eq!(ds.len(), 1, "one prim");
        ds.pop().unwrap()
    }

    /// Everything the schema and the radial-edge model require of the arrays, checked from the text.
    fn check_prim(text: &str) -> Doc {
        let d = read(text);
        let tol = d.f("brep:intersectTol3d").clone();
        let b = tol.len();
        let region_count = d.u("brep:regionCount");
        assert_eq!(region_count.len(), b);
        assert_eq!(d.f("brep:extent").len(), 6 * b);
        assert!(tol.iter().all(|t| *t > 0.0));
        let shell_count = d.u("region:shellCount");
        assert_eq!(shell_count.len(), sum(&region_count));
        assert_eq!(d.t("region:type").len(), shell_count.len());
        assert!(d.t("region:type").iter().all(|t| t == "voidRegion" || t == "solidRegion"));
        let fu_count = d.u("shell:faceuseCount");
        assert_eq!(fu_count.len(), sum(&shell_count));
        assert_eq!(d.t("shell:pointType").len(), fu_count.len());
        assert_eq!(d.u("shell:wireEdgeCount").len(), fu_count.len());
        let fu_face = d.u("faceuse:faceIndex");
        assert_eq!(fu_face.len(), sum(&fu_count));
        assert_eq!(d.t("faceuse:orientationType").len(), fu_face.len());
        let nf = fu_face.len() / 2;
        assert_eq!(fu_face.len(), 2 * nf);
        let loop_count = d.u("face:loopCount");
        assert_eq!(loop_count.len(), nf);
        assert!(loop_count.iter().all(|c| *c >= 1));
        assert_eq!(d.t("face:surfaceType").len(), nf);
        assert_eq!(d.t("face:trimType").len(), nf);
        let frange = d.f("face:range");
        assert_eq!(frange.len(), 4 * nf);
        for r in frange.chunks(4) {
            assert!(r[2] > r[0] + 1e-11 && r[3] > r[1] + 1e-11, "degenerate face range {r:?}");
        }
        let eu_count = d.u("loop:edgeuseCount");
        assert_eq!(eu_count.len(), sum(&loop_count));
        assert_eq!(d.u("loop:vertexIndex").len(), eu_count.len());
        let neu = sum(&eu_count);
        let eu_edge = d.u("edgeuse:edgeIndex");
        let eu_next = d.u("edgeuse:nextRadialEUIndex");
        let eu_orient = d.t("edgeuse:orientationType");
        let eu_entry = d.t("edgeuse:thisRadialEntryType");
        assert!([eu_edge.len(), eu_next.len(), eu_orient.len(), eu_entry.len()].iter().all(|n| *n == neu));
        let ne = d.t("edge:curveType").len();
        assert!(d.t("edge:curveType").iter().all(|t| t == "BrepCurve3dNurbAPI"));
        assert_eq!(d.f("edge:range").len(), 2 * ne);
        assert_eq!(d.f("edge:vertexIndices").len(), 2 * ne);
        let nv = d.t("vertex:pointType").len();
        assert_eq!(d.f("brep:vertexPoint:point:position").len(), 3 * nv);
        let cv_n = d.u("brep:edge3dNurb:curve3d:nurb:vertexCount");
        let cv_order = d.u("brep:edge3dNurb:curve3d:nurb:order");
        assert_eq!((cv_n.len(), cv_order.len()), (ne, ne));
        assert_eq!(d.f("brep:edge3dNurb:curve3d:nurb:controlVertices").len(), 3 * sum(&cv_n));
        assert_eq!(d.f("brep:edge3dNurb:curve3d:nurb:weights").len(), sum(&cv_n));
        assert_eq!(d.f("brep:edge3dNurb:curve3d:nurb:knots").len(), sum(&cv_n) + sum(&cv_order));
        // Geometry arrays: one entry per face of the type, and the API applied.
        let ft = d.t("face:surfaceType");
        let count = |t: &str| ft.iter().filter(|x| *x == t).count();
        if count("BrepSurfaceNurbAPI") > 0 {
            assert_eq!(d.u("brep:surface:nurb:uOrder").len(), count("BrepSurfaceNurbAPI"));
            let (nu, nv) = (d.u("brep:surface:nurb:uVertexCount"), d.u("brep:surface:nurb:vVertexCount"));
            let n: usize = nu.iter().zip(&nv).map(|(a, b)| a * b).sum();
            assert_eq!(d.v3("brep:surface:nurb:controlVertices").len(), n);
            assert_eq!(d.f("brep:surface:nurb:weights").len(), n);
        }
        for (t, p, n) in [
            ("BrepSurfacePlaneAPI", "plane", "origin"),
            ("BrepSurfaceCylinderAPI", "cylinder", "origin"),
            ("BrepSurfaceConeAPI", "cone", "origin"),
            ("BrepSurfaceSphereAPI", "sphere", "center"),
            ("BrepSurfaceTorusAPI", "torus", "origin"),
        ] {
            if count(t) > 0 {
                assert_eq!(d.v3(&format!("brep:surface:{p}:{n}")).len(), count(t));
                assert_eq!(d.v3(&format!("brep:surface:{p}:axis")).len(), count(t));
                assert_eq!(d.v3(&format!("brep:surface:{p}:refDirection")).len(), count(t));
                assert!(text.contains(&format!("\"{t}\"")));
            } else {
                assert!(!d.has(&format!("brep:surface:{p}:{n}")));
            }
        }
        // Partitions: each brep owns consecutive runs, and every index points inside its own.
        let region_starts = starts(&region_count);
        let shells_per_brep: Vec<usize> = (0..b).map(|i| sum(&shell_count[region_starts[i]..region_starts[i + 1]])).collect();
        let shell_starts = starts(&shells_per_brep);
        let fu_per_brep: Vec<usize> = (0..b).map(|i| sum(&fu_count[shell_starts[i]..shell_starts[i + 1]])).collect();
        let fu_starts = starts(&fu_per_brep);
        let face_starts: Vec<usize> = fu_starts.iter().map(|x| x / 2).collect();
        let loops_per_brep: Vec<usize> = (0..b).map(|i| sum(&loop_count[face_starts[i]..face_starts[i + 1]])).collect();
        let loop_starts = starts(&loops_per_brep);
        let eu_per_brep: Vec<usize> = (0..b).map(|i| sum(&eu_count[loop_starts[i]..loop_starts[i + 1]])).collect();
        let eu_starts = starts(&eu_per_brep);
        let vi = d.u("edge:vertexIndices");
        let mut edge_ranges = Vec::new();
        for i in 0..b {
            let es = &eu_edge[eu_starts[i]..eu_starts[i + 1]];
            let (lo, hi) = (*es.iter().min().unwrap(), *es.iter().max().unwrap());
            edge_ranges.push((lo, hi + 1));
            for e in lo..=hi {
                assert!(es.contains(&e), "edge {e} of brep {i} has no edgeuse");
            }
        }
        assert_eq!(edge_ranges[0].0, 0);
        for w in edge_ranges.windows(2) {
            assert_eq!(w[0].1, w[1].0, "edges of consecutive breps are consecutive");
        }
        assert_eq!(edge_ranges.last().unwrap().1, ne);
        let mut vert_ranges = Vec::new();
        for (i, (e0, e1)) in edge_ranges.iter().enumerate() {
            let vs: Vec<usize> = (*e0..*e1).flat_map(|e| [vi[2 * e], vi[2 * e + 1]]).collect();
            let (lo, hi) = (*vs.iter().min().unwrap(), *vs.iter().max().unwrap());
            for v in lo..=hi {
                assert!(vs.contains(&v), "vertex {v} of brep {i} is on no edge");
            }
            vert_ranges.push((lo, hi + 1));
        }
        assert_eq!(vert_ranges.last().unwrap().1, nv);
        // Faceuses: every face twice, once each side, inside its brep; side -> region.
        let fu_or = d.t("faceuse:orientationType");
        let mut side_region: HashMap<(usize, bool), usize> = HashMap::new();
        let mut shell_of_fu = Vec::new();
        for (s, c) in fu_count.iter().enumerate() {
            shell_of_fu.extend(std::iter::repeat_n(s, *c));
        }
        let region_of_shell: Vec<usize> = shell_count.iter().enumerate().flat_map(|(r, c)| std::iter::repeat_n(r, *c)).collect();
        for (k, f) in fu_face.iter().enumerate() {
            let brep = (0..b).find(|i| fu_starts[*i] <= k && k < fu_starts[i + 1]).unwrap();
            assert!(face_starts[brep] <= *f && *f < face_starts[brep + 1], "faceuse {k} points at face {f} outside brep {brep}");
            let top = fu_or[k] == "same";
            assert!(side_region.insert((*f, top), region_of_shell[shell_of_fu[k]]).is_none(), "face {f} side used twice");
        }
        assert_eq!(side_region.len(), 2 * nf);
        // Loops head to tail; the face of each edgeuse.
        let mut face_of_eu = Vec::new();
        let (mut eu, mut li) = (0, 0);
        for (f, lc) in loop_count.iter().enumerate() {
            for _ in 0..*lc {
                let n = eu_count[li];
                assert!(n >= 1);
                for k in 0..n {
                    let (e, nxt) = (eu_edge[eu + k], eu_edge[eu + (k + 1) % n]);
                    let end = if eu_orient[eu + k] == "same" { vi[2 * e + 1] } else { vi[2 * e] };
                    let start = if eu_orient[eu + (k + 1) % n] == "same" { vi[2 * nxt] } else { vi[2 * nxt + 1] };
                    assert_eq!(end, start, "loop {li} is not head to tail");
                    face_of_eu.push(f);
                }
                eu += n;
                li += 1;
            }
        }
        // Radial cycles: same edge, closed, orientations alternate, regions agree.
        for e in 0..neu {
            let mut cyc = vec![e];
            let mut c = eu_next[e];
            while c != e {
                cyc.push(c);
                c = eu_next[c];
                assert!(cyc.len() <= neu, "radial chain does not close");
            }
            assert!(cyc.iter().all(|x| eu_edge[*x] == eu_edge[e]), "radial chain leaves its edge");
            assert!(cyc.len() <= 2);
            // Individual edgeuses in walking order: (orientation is same, top side).
            let mut seq = Vec::new();
            for r in &cyc {
                let o = eu_orient[*r] == "same";
                if eu_entry[*r] == "topEntry" {
                    seq.extend([(o, true), (!o, false)]);
                } else {
                    seq.extend([(!o, false), (o, true)]);
                }
            }
            for k in 0..seq.len() {
                assert_ne!(seq[k].0, seq[(k + 1) % seq.len()].0, "orientations do not alternate round edge {}", eu_edge[e]);
            }
            if cyc.len() == 2 {
                for (k, r) in cyc.iter().enumerate() {
                    let nr = cyc[(k + 1) % 2];
                    let exit_top = eu_entry[*r] != "topEntry";
                    let enter_top = eu_entry[nr] == "topEntry";
                    assert_eq!(
                        side_region[&(face_of_eu[*r], exit_top)],
                        side_region[&(face_of_eu[nr], enter_top)],
                        "the walk leaves a face into another region than the next face is entered from"
                    );
                }
            }
        }
        // Edge ends agree with their vertices; control points and vertices are inside the extents.
        let cv = d.v3("brep:edge3dNurb:curve3d:nurb:controlVertices");
        let cv_starts = starts(&cv_n);
        let pos = d.v3("brep:vertexPoint:point:position");
        let ext = d.v3("brep:extent");
        for (e, (c, (t0, t1))) in edge_curves(&d).iter().enumerate() {
            assert!(c.valid(), "edge {e} curve");
            let brep = (0..b).find(|i| edge_ranges[*i].0 <= e && e < edge_ranges[*i].1).unwrap();
            for (t, v) in [(*t0, vi[2 * e]), (*t1, vi[2 * e + 1])] {
                let p = c.eval(t).unwrap();
                assert!(dist(p, pos[v]) <= tol[brep], "edge {e} end {p:?} is not vertex {v} {:?}", pos[v]);
            }
            let (lo, hi) = (ext[2 * brep], ext[2 * brep + 1]);
            for p in &cv[cv_starts[e]..cv_starts[e + 1]] {
                assert!((0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k]), "control point {p:?} outside the extent of brep {brep}");
            }
        }
        for (i, (v0, v1)) in vert_ranges.iter().enumerate() {
            for p in &pos[*v0..*v1] {
                assert!((0..3).all(|k| p[k] >= ext[2 * i][k] && p[k] <= ext[2 * i + 1][k]));
            }
        }
        // The prim's single-precision extent contains every brep extent.
        let prim = d.f("extent");
        assert_eq!(prim.len(), 6);
        for i in 0..b {
            for k in 0..3 {
                assert!(f64::from(prim[k] as f32) <= ext[2 * i][k] && f64::from(prim[3 + k] as f32) >= ext[2 * i + 1][k]);
            }
        }
        check_analytic(&d);
        d
    }

    /// Every analytic face: its loops lie on its surface (to the brep tolerance) and inside its
    /// parameter box, read back from the arrays.
    fn check_analytic(d: &Doc) {
        let tol = d.f("brep:intersectTol3d").iter().copied().fold(0.0, f64::max);
        let range = d.f("face:range");
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (f, ty) in d.t("face:surfaceType").iter().enumerate() {
            let (prefix, o_name, scalars): (&str, &str, &[&str]) = match ty.as_str() {
                "BrepSurfacePlaneAPI" => ("plane", "origin", &[]),
                "BrepSurfaceCylinderAPI" => ("cylinder", "origin", &["radius"]),
                "BrepSurfaceConeAPI" => ("cone", "origin", &["radius", "semiAngle"]),
                "BrepSurfaceSphereAPI" => ("sphere", "center", &["radius"]),
                "BrepSurfaceTorusAPI" => ("torus", "origin", &["majorRadius", "minorRadius"]),
                _ => continue,
            };
            let k = *seen.entry(ty.clone()).and_modify(|n| *n += 1).or_insert(0);
            let fr = Frame::new(
                d.v3(&format!("brep:surface:{prefix}:{o_name}"))[k],
                d.v3(&format!("brep:surface:{prefix}:axis"))[k],
                d.v3(&format!("brep:surface:{prefix}:refDirection"))[k],
            )
            .unwrap();
            let sc: Vec<f64> = scalars.iter().map(|s| d.f(&format!("brep:surface:{prefix}:{s}"))[k]).collect();
            let (u0, v0, u1, v1) = (range[4 * f], range[4 * f + 1], range[4 * f + 2], range[4 * f + 3]);
            for l in face_loops(d, f) {
                for p in l {
                    let q = fr.local(p);
                    let rho = q[0].hypot(q[1]);
                    let (u, v, off) = match prefix {
                        "plane" => (q[0], q[1], q[2].abs()),
                        "cylinder" => (q[1].atan2(q[0]), q[2], (rho - sc[0]).abs()),
                        "cone" => (q[1].atan2(q[0]), q[2], (rho - (sc[0] + q[2] * sc[1].tan())).abs() * sc[1].cos()),
                        "sphere" => (q[1].atan2(q[0]), q[2].atan2(rho), (norm(q) - sc[0]).abs()),
                        _ => (q[1].atan2(q[0]), q[2].atan2(rho - sc[0]), ((rho - sc[0]).hypot(q[2]) - sc[1]).abs()),
                    };
                    assert!(off <= tol, "face {f} ({ty}): a boundary point is {off} off its surface (tolerance {tol})");
                    // Angles repeat every turn; a point at a pole or apex has no u.
                    let angular_u = prefix != "plane" && rho > 1e-6 * (1.0 + sc.first().copied().unwrap_or(0.0));
                    let in_u = !angular_u || u1 - u0 >= TAU - 1e-9 || [-TAU, 0.0, TAU].iter().any(|s| u + s >= u0 - 1e-9 && u + s <= u1 + 1e-9);
                    assert!(in_u || prefix == "plane", "face {f} ({ty}): u {u} outside [{u0}, {u1}]");
                    let in_v = if prefix == "torus" {
                        v1 - v0 >= TAU - 1e-9 || [-TAU, 0.0, TAU].iter().any(|s| v + s >= v0 - 1e-9 && v + s <= v1 + 1e-9)
                    } else {
                        v >= v0 - 1e-9 && v <= v1 + 1e-9
                    };
                    assert!(in_v, "face {f} ({ty}): v {v} outside [{v0}, {v1}]");
                    if prefix == "cone" {
                        // No radius below the apex (SMLib builds a cone only from non-negative radii).
                        assert!(sc[0] + v0 * sc[1].tan() >= -1e-9 && sc[0] + v1 * sc[1].tan() >= -1e-9, "face {f}: the cone's range passes its apex");
                    }
                    if prefix == "plane" {
                        assert!(u >= u0 - 1e-9 && u <= u1 + 1e-9, "face {f}: u {u} outside [{u0}, {u1}]");
                    }
                }
            }
        }
    }

    /// Points along each loop of a face, in the direction the loop runs.
    fn face_loops(d: &Doc, face: usize) -> Vec<Vec<V3>> {
        let lc = d.u("face:loopCount");
        let ec = d.u("loop:edgeuseCount");
        let (eu_edge, eu_or) = (d.u("edgeuse:edgeIndex"), d.t("edgeuse:orientationType"));
        let curves = edge_curves(d);
        let first_loop = sum(&lc[..face]);
        let mut eu = sum(&ec[..first_loop]);
        let mut out = Vec::new();
        for &n in ec.iter().skip(first_loop).take(lc[face]) {
            let mut pts = Vec::new();
            for _ in 0..n {
                let (c, (a, b)) = &curves[eu_edge[eu]];
                let at = |i: usize| c.eval(a + (b - a) * i as f64 / 16.0).unwrap();
                if eu_or[eu] == "same" {
                    pts.extend((0..16).map(at));
                } else {
                    pts.extend((1..=16).rev().map(at));
                }
                eu += 1;
            }
            out.push(pts);
        }
        out
    }

    /// Twice the area vector of a polygon.
    fn newell(p: &[V3]) -> V3 {
        let mut n = [0.0; 3];
        for i in 0..p.len() {
            n = add(n, cross(p[i], p[(i + 1) % p.len()]));
        }
        n
    }

    /// The orientation of the face's faceuse in the first (infinite void) region's shell.
    fn outside_orientation(d: &Doc, face: usize) -> String {
        let n = d.u("shell:faceuseCount")[0];
        let k = d.u("faceuse:faceIndex")[..n].iter().position(|f| *f == face).unwrap();
        d.t("faceuse:orientationType")[k].clone()
    }

    /// Plane faces of a single convex-ish solid: the outside faceuse is `same` exactly when the
    /// plane's axis points away from the body's middle, and every outer loop runs counter-clockwise
    /// about the axis (inner loops clockwise); the parameter box matches the face.
    fn check_planes(d: &Doc) {
        let pos = d.v3("brep:vertexPoint:point:position");
        let mid = mul(pos.iter().fold([0.0; 3], |a, p| add(a, *p)), 1.0 / pos.len() as f64);
        let axes = d.v3("brep:surface:plane:axis");
        let range = d.f("face:range");
        let lc = d.u("face:loopCount");
        let mut k = 0;
        for (f, t) in d.t("face:surfaceType").iter().enumerate() {
            if t != "BrepSurfacePlaneAPI" {
                continue;
            }
            let axis = axes[k];
            k += 1;
            let loops = face_loops(d, f);
            let c = mul(loops[0].iter().fold([0.0; 3], |a, p| add(a, *p)), 1.0 / loops[0].len() as f64);
            // Only meaningful where the body is convex about its middle (boxes, cylinders).
            if lc.iter().all(|n| *n == 1) {
                let away = dot(axis, sub(c, mid)) > 0.0;
                assert_eq!(outside_orientation(d, f) == "same", away, "face {f}: outside faceuse against axis {axis:?}");
            }
            let outer = dot(newell(&loops[0]), axis);
            assert!(outer > 0.0, "face {f}: outer loop is not counter-clockwise about its axis");
            for inner in &loops[1..] {
                assert!(dot(newell(inner), axis) < 0.0, "face {f}: an inner loop is not clockwise");
            }
            // The box is at least the polygon's area and not much more.
            let area = outer / 2.0 - loops[1..].iter().map(|l| dot(newell(l), axis).abs() / 2.0).sum::<f64>();
            let boxed = (range[4 * f + 2] - range[4 * f]) * (range[4 * f + 3] - range[4 * f + 1]);
            assert!(boxed >= area * 0.999, "face {f}: box {boxed} smaller than its area {area}");
        }
    }

    fn plate() -> Body {
        box_solid(Vec3::ZERO, Vec3::new(40.0, 30.0, 10.0)).unwrap()
    }

    #[test]
    fn a_box_is_six_planes_twelve_edges_and_twenty_four_edgeuses() {
        let b = box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap();
        let t = export(&[("box", &b)]);
        let d = check(&t);
        assert_eq!(d.u("brep:regionCount"), [2]);
        assert_eq!(d.t("region:type"), &["voidRegion", "solidRegion"]);
        assert_eq!(d.u("region:shellCount"), [1, 1]);
        assert_eq!(d.u("shell:faceuseCount"), [6, 6]);
        assert_eq!(d.t("face:surfaceType").len(), 6);
        assert!(d.t("face:surfaceType").iter().all(|t| t == "BrepSurfacePlaneAPI"));
        assert_eq!(d.t("edge:curveType").len(), 12);
        assert_eq!(d.t("vertex:pointType").len(), 8);
        assert_eq!(d.u("edgeuse:edgeIndex").len(), 24);
        // Every edge is a degree-1 two-point curve.
        assert!(d.u("brep:edge3dNurb:curve3d:nurb:order").iter().all(|o| *o == 2));
        assert!(d.u("brep:edge3dNurb:curve3d:nurb:vertexCount").iter().all(|n| *n == 2));
        assert!(t.contains("prepend apiSchemas = [\"BrepPointAPI:vertexPoint\", \"BrepCurve3dNurbAPI:edge3dNurb\", \"BrepSurfacePlaneAPI\"]"));
        check_planes(&d);
        // The extent is the box padded by the tolerance.
        let (ext, tol) = (d.v3("brep:extent"), d.f("brep:intersectTol3d")[0]);
        for k in 0..3 {
            let (lo, hi) = (0.0, [10.0, 20.0, 30.0][k]);
            assert!((ext[0][k] - (lo - tol)).abs() < 1e-9 && (ext[1][k] - (hi + tol)).abs() < 1e-9, "{ext:?}");
        }
    }

    #[test]
    fn a_cylinder_has_a_seam_and_a_full_turn_of_u() {
        let b = cylinder(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 1.0, 1.0), 5.0, 12.0).unwrap();
        let d = check(&export(&[("cyl", &b)]));
        let ft = d.t("face:surfaceType");
        assert_eq!(ft.iter().filter(|t| *t == "BrepSurfacePlaneAPI").count(), 2);
        let f = ft.iter().position(|t| t == "BrepSurfaceCylinderAPI").unwrap();
        assert_eq!(d.f("brep:surface:cylinder:radius").len(), 1);
        assert!((d.f("brep:surface:cylinder:radius")[0] - 5.0).abs() < 1e-9);
        let r = &d.f("face:range")[4 * f..4 * f + 4];
        assert!(r[0].abs() < 1e-12 && (r[2] - TAU).abs() < 1e-12, "u range {r:?}");
        assert!((r[3] - r[1] - 12.0).abs() < 0.01, "v range {r:?}");
        // Outside of a solid cylinder is the normal side.
        assert_eq!(outside_orientation(&d, f), "same");
        // The seam is the one edge both of whose edgeuses belong to the lateral face (opposite ways).
        let (eu_edge, eu_or) = (d.u("edgeuse:edgeIndex"), d.t("edgeuse:orientationType"));
        let ec = d.u("loop:edgeuseCount");
        let lc = d.u("face:loopCount");
        let first = sum(&ec[..sum(&lc[..f])]);
        let mine = &eu_edge[first..first + ec[sum(&lc[..f])]];
        let seam: Vec<usize> = (0..mine.len()).filter(|i| mine.iter().filter(|e| **e == mine[*i]).count() == 2).collect();
        assert_eq!(seam.len(), 2, "a seam is used twice by the lateral face");
        assert_ne!(eu_or[first + seam[0]], eu_or[first + seam[1]]);
        check_planes(&d);
    }

    #[test]
    fn spheres_and_tori_keep_their_analytic_surfaces() {
        let d = check(&export(&[("sphere", &sphere(Vec3::new(1.0, 2.0, 3.0), 10.0).unwrap())]));
        assert!(d.t("face:surfaceType").iter().all(|t| t == "BrepSurfaceSphereAPI"));
        assert!(d.f("brep:surface:sphere:radius").iter().all(|r| (r - 10.0).abs() < 1e-9));
        for r in d.f("face:range").chunks(4) {
            assert!(r[1] >= -FRAC_PI_2 && r[3] <= FRAC_PI_2 && r[2] - r[0] <= TAU + 1e-9, "{r:?}");
        }
        let d = check(&export(&[("torus", &torus(Vec3::ZERO, 20.0, 4.0).unwrap())]));
        assert!(d.t("face:surfaceType").iter().all(|t| t == "BrepSurfaceTorusAPI"));
        assert!(d.f("brep:surface:torus:majorRadius").iter().all(|r| (r - 20.0).abs() < 1e-9));
        assert!(d.f("brep:surface:torus:minorRadius").iter().all(|r| (r - 4.0).abs() < 1e-6));
        for r in d.f("face:range").chunks(4) {
            assert!(r[2] - r[0] <= TAU + 1e-6 && r[3] - r[1] <= TAU + 1e-6, "{r:?}");
        }
    }

    #[test]
    fn a_plate_with_a_hole_has_a_two_loop_face() {
        let pin = cylinder(Vec3::new(20.0, 15.0, -1.0), Vec3::Z, 5.0, 12.0).unwrap();
        let holed = boolean(&plate(), &pin, BoolOp::Cut).unwrap().unwrap();
        let d = check(&export(&[("holed", &holed)]));
        let lc = d.u("face:loopCount");
        assert_eq!(lc.iter().filter(|n| **n == 2).count(), 2, "top and bottom have the hole as an inner loop");
        assert_eq!(lc.len(), 7);
        // The hole's wall faces the axis: the outside faceuse is the one against the surface normal.
        let f = d.t("face:surfaceType").iter().position(|t| t == "BrepSurfaceCylinderAPI").unwrap();
        assert_eq!(outside_orientation(&d, f), "opposite");
        check_planes(&d);
    }

    #[test]
    fn each_body_is_its_own_prim_named_after_it() {
        let a = box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap();
        let b = cylinder(Vec3::new(50.0, 0.0, 0.0), Vec3::Z, 5.0, 12.0).unwrap();
        let t = export(&[("a", &a), ("b", &b)]);
        let ds = check_all(&t);
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].u("brep:regionCount"), [2]);
        assert_eq!(ds[1].u("shell:faceuseCount"), [3, 3]);
        // Indices start at zero in each prim.
        assert!(ds[1].u("faceuse:faceIndex").iter().all(|f| *f < 3));
        let ps = prims(&t);
        assert!(ps[0].starts_with("    def BrepArray \"a\" ("), "{}", &ps[0][..80]);
        assert!(ps[1].starts_with("    def BrepArray \"b\" ("));
        assert!(ps[0].contains("dictionary solidcraft = {\n                string name = \"a\"\n"));
        assert!(!t.contains("primvars:displayColor"));
    }

    #[test]
    fn names_become_identifiers_and_colours_become_display_colour() {
        let mut used = HashSet::new();
        let ids: Vec<String> =
            ["Bracket left 1", "1st-part", "", "x", "x", "x", "Bracket_left_1", "Caf\u{e9}"].iter().map(|n| identifier(n, &mut used)).collect();
        assert_eq!(ids, ["Bracket_left_1", "_1st_part", "Body", "x", "x_2", "x_3", "Bracket_left_1_2", "Caf_"]);
        assert_eq!(quoted("a\"b\\c\nd"), "\"a\\\"b\\\\c d\"");
        let a = box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap();
        let b = cylinder(Vec3::new(50.0, 0.0, 0.0), Vec3::Z, 5.0, 12.0).unwrap();
        let bodies = [
            ExportBody { name: "Bracket \"left\" 1".into(), body: &a, color: Some([1.0, 0.5, 0.0]) },
            ExportBody { name: "Bracket \"left\" 1".into(), body: &b, color: None },
        ];
        let t = usd_export_bodies(&bodies, &StepHeader::default()).unwrap();
        check_all(&t);
        let ps = prims(&t);
        assert!(ps[0].starts_with("    def BrepArray \"Bracket__left__1\" ("));
        assert!(ps[1].starts_with("    def BrepArray \"Bracket__left__1_2\" ("));
        assert!(ps[0].contains("string name = \"Bracket \\\"left\\\" 1\"\n"));
        assert!(ps[0].contains(
            "        color3f[] primvars:displayColor = [(1.0, 0.5, 0.0)]\n        uniform token primvars:displayColor:interpolation = \"constant\"\n"
        ));
        assert!(!ps[1].contains("displayColor"));
    }

    #[test]
    fn filleted_and_booleaned_bodies_export() {
        let rounded = fillet(&plate(), &[Vec3::new(20.0, 0.0, 10.0)], 3.0).unwrap();
        let d = check(&export(&[("rounded", &rounded)]));
        assert!(d.t("face:surfaceType").len() > 6);
    }

    /// Bodies the builders make from sketches: revolved (tori, cones, a horn torus), tapered
    /// and lofted. Whatever surface kinds STEP carries must come out as valid arrays.
    #[test]
    fn sketch_built_bodies_export() {
        use crate::{extrude_tapered, loft, revolve};
        use solidcraft_geom::{Loop2, Plane, Region2, Seg2, Vec2};
        let tau = TAU;
        let ring = Region2 { outer: Loop2::circle(Vec2::new(10.0, 0.0), 3.0), holes: vec![] };
        let donut = revolve(&Plane::XZ, &[ring], Vec2::ZERO, Vec2::Y, 1.5).unwrap().pop().unwrap();
        let d = check(&export(&[("donut", &donut)]));
        assert!(d.t("face:surfaceType").iter().any(|t| t == "BrepSurfaceTorusAPI"));
        let prof =
            Region2 { outer: Loop2::polygon(&[Vec2::new(0.0, 4.0), Vec2::new(3.0, 5.0), Vec2::new(3.0, 12.0), Vec2::new(0.0, 12.0)]), holes: vec![] };
        let frustum = revolve(&Plane::XZ, &[prof], Vec2::ZERO, Vec2::Y, tau).unwrap().pop().unwrap();
        let d = check(&export(&[("frustum", &frustum)]));
        assert!(d.t("face:surfaceType").iter().any(|t| t == "BrepSurfaceConeAPI"));
        for (k, t) in d.t("face:surfaceType").iter().enumerate() {
            if t == "BrepSurfaceConeAPI" {
                let r = &d.f("face:range")[4 * k..4 * k + 4];
                assert!(r[0].abs() < 1e-12 && (r[2] - tau).abs() < 1e-12, "{r:?}");
            }
        }
        for a in d.f("brep:surface:cone:semiAngle") {
            assert!(*a > 0.0 && *a < FRAC_PI_2);
        }
        let horn = Region2 { outer: Loop2::circle(Vec2::new(5.0, 0.0), 5.0), holes: vec![] };
        let horn = revolve(&Plane::XZ, &[horn], Vec2::ZERO, Vec2::Y, FRAC_PI_2).unwrap().pop().unwrap();
        check(&export(&[("horn", &horn)]));
        let disc = Region2 { outer: Loop2::circle(Vec2::ZERO, 6.0), holes: vec![] };
        let tapered = extrude_tapered(&Plane::XY, &disc, 10.0, 1.0, 0.1).unwrap();
        let d = check(&export(&[("tapered", &tapered)]));
        assert!(d.t("face:surfaceType").iter().any(|t| t == "BrepSurfaceNurbAPI"));
        // A profile with a free-form curve is a true surface of revolution (not a cylinder or cone).
        let blob = Loop2 {
            segs: vec![
                Seg2::Cubic { p0: Vec2::new(40.0, 0.0), p1: Vec2::new(50.0, -5.0), p2: Vec2::new(60.0, 5.0), p3: Vec2::new(70.0, 0.0) },
                Seg2::Line { a: Vec2::new(70.0, 0.0), b: Vec2::new(70.0, 10.0) },
                Seg2::Conic { a: Vec2::new(70.0, 10.0), apex: Vec2::new(55.0, 25.0), b: Vec2::new(40.0, 10.0), w: 0.7 },
                Seg2::Line { a: Vec2::new(40.0, 10.0), b: Vec2::new(40.0, 0.0) },
            ],
        };
        let wavy = revolve(&Plane::XZ, &[Region2 { outer: blob, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap();
        let d = check(&export(&[("wavy", &wavy)]));
        assert!(d.t("face:surfaceType").iter().any(|t| t == "BrepSurfaceNurbAPI"));
        let top = Plane { origin: Vec3::new(0.0, 0.0, 10.0), ..Plane::XY };
        let lofted = loft(&[(Plane::XY, Loop2::circle(Vec2::ZERO, 6.0)), (top, Loop2::circle(Vec2::ZERO, 3.0))]).unwrap();
        check(&export(&[("lofted", &lofted)]));
    }

    /// Faces that reach a pole or an apex: a sphere turned from a half disc and a cone from a
    /// triangle. Their seams and singular points still give a full turn of u.
    #[test]
    fn poles_and_apexes_keep_a_full_turn() {
        use crate::revolve;
        use solidcraft_geom::{Loop2, Plane, Region2, Seg2, Vec2};
        let half = Loop2 {
            segs: vec![
                Seg2::Arc { center: Vec2::ZERO, radius: 5.0, start: -FRAC_PI_2, sweep: PI },
                Seg2::Line { a: Vec2::new(0.0, 5.0), b: Vec2::new(0.0, -5.0) },
            ],
        };
        let ball = revolve(&Plane::XZ, &[Region2 { outer: half, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap();
        let d = check(&export(&[("ball", &ball)]));
        let k = d.t("face:surfaceType").iter().position(|t| t == "BrepSurfaceSphereAPI").expect("a spherical face");
        let r = &d.f("face:range")[4 * k..4 * k + 4];
        assert!(r[0].abs() < 1e-9 && (r[2] - TAU).abs() < 1e-9, "u range {r:?}");
        // The face with the north pole: it reaches it and stays inside the latitudes. (The other
        // face is the south cap, in a frame of its own, as a patch about the equator.)
        assert!(r[3] >= FRAC_PI_2 - 1e-3 && r[3] <= FRAC_PI_2 && r[1] >= -FRAC_PI_2, "v range {r:?}");
        assert_eq!(outside_orientation(&d, k), "same");
        let tri = Loop2::polygon(&[Vec2::ZERO, Vec2::new(5.0, 0.0), Vec2::new(0.0, 10.0)]);
        let cone = revolve(&Plane::XZ, &[Region2 { outer: tri, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap();
        let d = check(&export(&[("cone", &cone)]));
        let k = d.t("face:surfaceType").iter().position(|t| t == "BrepSurfaceConeAPI").expect("a conical face");
        let r = &d.f("face:range")[4 * k..4 * k + 4];
        assert!(r[0].abs() < 1e-9 && (r[2] - TAU).abs() < 1e-9, "u range {r:?}");
    }

    #[test]
    fn swept_helical_and_shelled_bodies_export() {
        use crate::{PathSeg, shell, sweep, sweep_helix};
        use solidcraft_geom::{Loop2, Plane, Region2, Vec2};
        let disc = Region2 { outer: Loop2::circle(Vec2::ZERO, 2.0), holes: vec![] };
        let path = [
            PathSeg::Line { a: Vec3::ZERO, b: Vec3::new(0.0, 0.0, 20.0) },
            PathSeg::Arc { a: Vec3::new(0.0, 0.0, 20.0), center: Vec3::new(10.0, 0.0, 20.0), axis: Vec3::Y, angle: -1.5 },
        ];
        check(&export(&[("pipe", &sweep(&Plane::XY, &disc, &path).unwrap())]));
        let wire = Region2 { outer: Loop2::circle(Vec2::new(8.0, 0.0), 1.0), holes: vec![] };
        check(&export(&[("spring", &sweep_helix(&Plane::XZ, &wire, Vec3::ZERO, Vec3::Z, 5.0, 3.0).unwrap())]));
        let blk = box_solid(Vec3::ZERO, Vec3::new(20.0, 20.0, 20.0)).unwrap();
        check(&export(&[("cup", &shell(&blk, &[Vec3::new(10.0, 10.0, 20.0)], 2.0).unwrap())]));
    }

    /// A body with an internal cavity: the infinite void, the solid with two shells, the cavity.
    #[test]
    fn a_cavity_gets_its_own_void_region() {
        let blk = box_solid(Vec3::ZERO, Vec3::new(20.0, 20.0, 20.0)).unwrap();
        let inner = box_solid(Vec3::new(5.0, 5.0, 5.0), Vec3::new(15.0, 15.0, 15.0)).unwrap();
        let hollow = boolean(&blk, &inner, BoolOp::Cut).unwrap().unwrap();
        let d = check(&export(&[("hollow", &hollow)]));
        assert_eq!(d.u("brep:regionCount"), [3]);
        assert_eq!(d.t("region:type"), &["voidRegion", "solidRegion", "voidRegion"]);
        assert_eq!(d.u("region:shellCount"), [1, 2, 1]);
        assert_eq!(d.u("shell:faceuseCount"), [6, 6, 6, 6]);
        // The cavity's faceuses face into the cavity: `same` exactly when the axis points to its middle.
        let fu_face = d.u("faceuse:faceIndex");
        let fu_or = d.t("faceuse:orientationType");
        let axes = d.v3("brep:surface:plane:axis");
        for k in 18..24 {
            let f = fu_face[k];
            let c = face_loops(&d, f)[0].iter().fold([0.0; 3], |a, p| add(a, *p));
            let c = mul(c, 1.0 / face_loops(&d, f)[0].len() as f64);
            let toward = dot(axes[f], sub([10.0; 3], c)) > 0.0;
            assert_eq!(fu_or[k] == "same", toward, "cavity faceuse {k}");
        }
        // A spherical cavity too.
        let ball = boolean(&blk, &sphere(Vec3::new(10.0, 10.0, 10.0), 4.0).unwrap(), BoolOp::Cut).unwrap().unwrap();
        let d = check(&export(&[("ball", &ball)]));
        assert_eq!(d.u("region:shellCount"), [1, 2, 1]);
    }

    /// Bodies covering every kind of face and shell the exporter writes.
    fn sample_bodies() -> Vec<(&'static str, Body)> {
        use crate::{extrude_tapered, loft, revolve};
        use solidcraft_geom::{Loop2, Plane, Region2, Seg2, Vec2};
        let blk = box_solid(Vec3::ZERO, Vec3::new(20.0, 20.0, 20.0)).unwrap();
        let pin = cylinder(Vec3::new(20.0, 15.0, -1.0), Vec3::Z, 5.0, 12.0).unwrap();
        let wave = Loop2 {
            segs: vec![
                Seg2::Cubic { p0: Vec2::new(40.0, 0.0), p1: Vec2::new(50.0, -5.0), p2: Vec2::new(60.0, 5.0), p3: Vec2::new(70.0, 0.0) },
                Seg2::Line { a: Vec2::new(70.0, 0.0), b: Vec2::new(70.0, 10.0) },
                Seg2::Conic { a: Vec2::new(70.0, 10.0), apex: Vec2::new(55.0, 25.0), b: Vec2::new(40.0, 10.0), w: 0.7 },
                Seg2::Line { a: Vec2::new(40.0, 10.0), b: Vec2::new(40.0, 0.0) },
            ],
        };
        let ring = Region2 { outer: Loop2::circle(Vec2::new(10.0, 0.0), 3.0), holes: vec![] };
        let disc = Region2 { outer: Loop2::circle(Vec2::ZERO, 6.0), holes: vec![] };
        let top = Plane { origin: Vec3::new(0.0, 0.0, 10.0), ..Plane::XY };
        let half_disc = Loop2 {
            segs: vec![
                Seg2::Arc { center: Vec2::ZERO, radius: 5.0, start: -FRAC_PI_2, sweep: PI },
                Seg2::Line { a: Vec2::new(0.0, 5.0), b: Vec2::new(0.0, -5.0) },
            ],
        };
        let quarter_disc = Loop2 {
            segs: vec![
                Seg2::Line { a: Vec2::ZERO, b: Vec2::new(5.0, 0.0) },
                Seg2::Arc { center: Vec2::ZERO, radius: 5.0, start: 0.0, sweep: FRAC_PI_2 },
                Seg2::Line { a: Vec2::new(0.0, 5.0), b: Vec2::ZERO },
            ],
        };
        let triangle = Loop2::polygon(&[Vec2::ZERO, Vec2::new(5.0, 0.0), Vec2::new(0.0, 10.0)]);
        let trapezoid = Loop2::polygon(&[Vec2::new(0.0, 4.0), Vec2::new(3.0, 5.0), Vec2::new(3.0, 12.0), Vec2::new(0.0, 12.0)]);
        vec![
            ("box", box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap()),
            ("cylinder", cylinder(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 1.0, 1.0), 5.0, 12.0).unwrap()),
            ("sphere", sphere(Vec3::new(1.0, 2.0, 3.0), 10.0).unwrap()),
            ("torus", torus(Vec3::ZERO, 20.0, 4.0).unwrap()),
            ("holed", boolean(&plate(), &pin, BoolOp::Cut).unwrap().unwrap()),
            ("rounded", fillet(&plate(), &[Vec3::new(20.0, 0.0, 10.0)], 3.0).unwrap()),
            ("cavity", boolean(&blk, &box_solid(Vec3::new(5.0, 5.0, 5.0), Vec3::new(15.0, 15.0, 15.0)).unwrap(), BoolOp::Cut).unwrap().unwrap()),
            ("donut", revolve(&Plane::XZ, &[ring], Vec2::ZERO, Vec2::Y, 1.5).unwrap().pop().unwrap()),
            ("wavy", revolve(&Plane::XZ, &[Region2 { outer: wave, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap()),
            ("tapered", extrude_tapered(&Plane::XY, &disc, 10.0, 1.0, 0.1).unwrap()),
            ("lofted", loft(&[(Plane::XY, Loop2::circle(Vec2::ZERO, 6.0)), (top, Loop2::circle(Vec2::ZERO, 3.0))]).unwrap()),
            ("ball", revolve(&Plane::XZ, &[Region2 { outer: half_disc, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap()),
            ("dome", revolve(&Plane::XZ, &[Region2 { outer: quarter_disc, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap()),
            ("cone", revolve(&Plane::XZ, &[Region2 { outer: triangle, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap()),
            ("frustum", revolve(&Plane::XZ, &[Region2 { outer: trapezoid, holes: vec![] }], Vec2::ZERO, Vec2::Y, TAU).unwrap().pop().unwrap()),
            (
                "bowl",
                boolean(
                    &sphere(Vec3::ZERO, 10.0).unwrap(),
                    &box_solid(Vec3::new(-20.0, -20.0, 3.0), Vec3::new(20.0, 20.0, 20.0)).unwrap(),
                    BoolOp::Cut,
                )
                .unwrap()
                .unwrap(),
            ),
            (
                "bored_ball",
                boolean(&sphere(Vec3::ZERO, 10.0).unwrap(), &cylinder(Vec3::new(2.0, 1.0, -20.0), Vec3::Z, 3.0, 40.0).unwrap(), BoolOp::Cut)
                    .unwrap()
                    .unwrap(),
            ),
            (
                "cut_ring",
                boolean(
                    &torus(Vec3::ZERO, 20.0, 4.0).unwrap(),
                    &box_solid(Vec3::new(-5.0, -30.0, -10.0), Vec3::new(5.0, 30.0, 10.0)).unwrap(),
                    BoolOp::Cut,
                )
                .unwrap()
                .unwrap(),
            ),
        ]
    }

    /// The whole way round: what the importer reads back from the export has the volume and
    /// area of the original, so no solid comes out inside out.
    #[test]
    fn exported_bodies_read_back_with_the_same_volume() {
        use crate::{measure, step_import};
        let rel = |a: f64, b: f64| (a - b).abs() / b.abs().max(1e-9);
        let bodies = sample_bodies();
        for (name, b) in &bodies {
            let text = export(&[(name, b)]);
            check(&text);
            let (step, warnings) = crate::usd::usd_in::usd_to_step(&text).unwrap();
            let imp = step_import(&step).unwrap();
            assert!(warnings.is_empty() && imp.warnings.is_empty(), "{name}: {warnings:?} {:?}", imp.warnings);
            // A body in two pieces (the cut ring) may come back as its two bodies: compare the whole.
            assert!(!imp.bodies.is_empty(), "{name}");
            let m0 = measure(b).unwrap();
            let (volume, area) = imp.bodies.iter().fold((0.0, 0.0), |(v, a), x| {
                let m = measure(&x.body).unwrap();
                (v + m.volume, a + m.area)
            });
            assert!(rel(volume, m0.volume) < 2e-3, "{name}: volume {volume} vs {}", m0.volume);
            assert!(rel(area, m0.area) < 2e-3, "{name}: area {area} vs {}", m0.area);
            assert!(imp.bodies.iter().all(|x| x.closed), "{name} reads back closed");
        }
    }

    /// A reader that works trims out from the 3D edges (NVIDIA's SMLib does, for analytic faces)
    /// finds no region inside a loop that winds round a pole without a seam, or that is the
    /// outside of a hole: it read the exported sphere with a fifth of its volume and the torus
    /// with none. So the outer loop of a sphere or torus face closes in its parameter box: when it
    /// turns round a period it runs a seam both ways, and otherwise it runs counter-clockwise.
    #[test]
    fn sphere_and_torus_faces_close_in_their_parameter_box() {
        let mut seams = 0;
        for (name, body) in sample_bodies() {
            if !["sphere", "torus", "ball", "dome", "bowl", "bored_ball", "cut_ring"].contains(&name) {
                continue;
            }
            let d = check(&export(&[(name, &body)]));
            let (eu_edge, eu_or) = (d.u("edgeuse:edgeIndex"), d.t("edgeuse:orientationType"));
            let (lc, ec) = (d.u("face:loopCount"), d.u("loop:edgeuseCount"));
            let mut kinds: HashMap<String, usize> = HashMap::new();
            for (f, ty) in d.t("face:surfaceType").iter().enumerate() {
                let k = *kinds.entry(ty.clone()).and_modify(|n| *n += 1).or_insert(0);
                let (prefix, o_name) = match ty.as_str() {
                    "BrepSurfaceSphereAPI" => ("sphere", "center"),
                    "BrepSurfaceTorusAPI" => ("torus", "origin"),
                    _ => continue,
                };
                let fr = Frame::new(
                    d.v3(&format!("brep:surface:{prefix}:{o_name}"))[k],
                    d.v3(&format!("brep:surface:{prefix}:axis"))[k],
                    d.v3(&format!("brep:surface:{prefix}:refDirection"))[k],
                )
                .unwrap();
                let first_loop = sum(&lc[..f]);
                let first_eu = sum(&ec[..first_loop]);
                let uses: Vec<(usize, bool)> = (first_eu..first_eu + ec[first_loop]).map(|e| (eu_edge[e], eu_or[e] == "same")).collect();
                let seam = uses.iter().any(|(e, fwd)| uses.contains(&(*e, !*fwd)));
                // The outer loop in the surface's parameters.
                let pts = face_loops(&d, f).swap_remove(0);
                let (big, singular) = (if prefix == "torus" { d.f("brep:surface:torus:majorRadius")[k] } else { 0.0 }, 1e-9);
                let uv: Vec<(Option<f64>, f64)> = pts
                    .iter()
                    .map(|p| {
                        let q = fr.local(*p);
                        let rho = q[0].hypot(q[1]);
                        ((rho > singular).then(|| q[1].atan2(q[0])), q[2].atan2(rho - big))
                    })
                    .collect();
                let kept: Vec<(f64, f64)> = uv.iter().filter_map(|(u, v)| u.map(|u| (u, *v))).collect();
                let (u, turns_u) = unwrap_angles(&kept.iter().map(|x| Some(x.0)).collect::<Vec<_>>());
                let (v, turns_v) = if prefix == "torus" {
                    unwrap_angles(&kept.iter().map(|x| Some(x.1)).collect::<Vec<_>>())
                } else {
                    (kept.iter().map(|x| x.1).collect(), 0.0)
                };
                if turns_u != 0.0 || turns_v != 0.0 {
                    assert!(seam, "{name}: face {f} ({prefix}) has a loop turning {turns_u}/{turns_v} times round with no seam");
                } else {
                    let pairs: Vec<(f64, f64)> = u.iter().copied().zip(v.iter().copied()).collect();
                    let area: f64 = pairs.iter().zip(pairs.iter().cycle().skip(1)).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum::<f64>() / 2.0;
                    assert!(area > 0.0, "{name}: the outer loop of face {f} ({prefix}) is clockwise (the outside of a hole), area {area}");
                }
                if seam {
                    seams += 1;
                    // The seam edges are the ones used twice; on a torus the loop is the box: a
                    // meridian and a parallel through one vertex, each closed.
                    if prefix == "torus" {
                        assert_eq!(uses.len(), 4, "{name}: a torus box has four sides");
                        let vi = d.u("edge:vertexIndices");
                        assert!(uses.iter().all(|(e, _)| vi[2 * e] == vi[2 * e + 1]), "{name}: the torus seams are closed edges");
                        assert_eq!(lc[f], 2, "{name}: the old loop stays as a hole");
                    }
                }
            }
        }
        assert!(seams >= 4, "the sphere, ball, torus and others get seams ({seams})");
    }

    /// A body's name is text, not a number: "Info", "Reinforcement" and "NaN part" hold "inf" and
    /// "NaN" and must not look like a number that is not finite.
    #[test]
    fn names_that_spell_nan_or_inf_export() {
        let b = box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap();
        let names = ["Info", "Reinforcement", "NaN part", "Infill", "inf", "nan"];
        let bodies: Vec<(&str, &Body)> = names.iter().map(|n| (*n, &b)).collect();
        let t = export(&bodies);
        let ds = check_all(&t);
        assert_eq!(ds.len(), names.len());
        let ps = prims(&t);
        for (p, n) in ps.iter().zip(names) {
            assert!(p.contains(&format!("string name = {}", quoted(n))), "{n}");
        }
        assert!(ps[2].starts_with("    def BrepArray \"NaN_part\" ("));
    }

    /// Each body is exported on its own, so its prim has its own pieces, name and colour: a body
    /// in two pieces next to a single one is two prims (the first with two breps), not three.
    #[test]
    fn a_body_in_two_pieces_stays_one_prim_with_its_own_colour() {
        let a = box_solid(Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0)).unwrap();
        let b = box_solid(Vec3::new(50.0, 0.0, 0.0), Vec3::new(60.0, 10.0, 10.0)).unwrap();
        let pair = crate::union_apart(&[a.clone(), b]).unwrap().expect("two boxes far apart are one body of two pieces");
        let bodies = [
            ExportBody { name: "Pair".into(), body: &pair, color: Some([1.0, 0.0, 0.0]) },
            ExportBody { name: "Single".into(), body: &a, color: Some([0.0, 0.0, 1.0]) },
        ];
        let t = usd_export_bodies(&bodies, &StepHeader::default()).unwrap();
        let ds = check_all(&t);
        let ps = prims(&t);
        assert_eq!(ds.len(), 2, "{t}");
        assert!(ps[0].starts_with("    def BrepArray \"Pair\" (") && ps[1].starts_with("    def BrepArray \"Single\" ("));
        assert_eq!(ds[0].u("brep:regionCount"), [2, 2]);
        assert_eq!(ds[1].u("brep:regionCount"), [2]);
        assert!(ps[0].contains("primvars:displayColor = [(1.0, 0.0, 0.0)]"));
        assert!(ps[1].contains("primvars:displayColor = [(0.0, 0.0, 1.0)]"));
    }

    #[test]
    fn nurbs_arcs_lie_on_the_circle_and_reverse_cleanly() {
        let f = Frame::new([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]).unwrap();
        for sweep in [0.3, FRAC_PI_2, 2.0, PI, 5.0, TAU, -1.0, -TAU] {
            let c = Nurbs::arc(&f, 4.0, 4.0, 0.7, sweep);
            assert!(c.valid());
            let (a, b) = c.domain().unwrap();
            for i in 0..=40 {
                let p = c.eval(a + (b - a) * i as f64 / 40.0).unwrap();
                assert!((dist(p, f.o) - 4.0).abs() < 1e-12 && (p[2] - 3.0).abs() < 1e-12);
            }
            let (s, e) = (c.eval(a).unwrap(), c.eval(b).unwrap());
            let r = c.reversed();
            assert!(dist(r.eval(a).unwrap(), e) < 1e-12 && dist(r.eval(b).unwrap(), s) < 1e-12);
            let mid = c.eval((a + b) / 2.0).unwrap();
            assert!(dist(r.eval((a + b) / 2.0).unwrap(), mid) < 1e-12);
            // The end of the sweep is where the angle says.
            let ang = 0.7 + sweep;
            assert!(dist(e, f.at(4.0 * ang.cos(), 4.0 * ang.sin(), 0.0)) < 1e-12);
            let t = c.nearest(mid).unwrap();
            assert!(dist(c.eval(t).unwrap(), mid) < 1e-9);
        }
        let l = Nurbs::line([0.0; 3], [2.0, 0.0, 0.0]);
        assert_eq!(l.eval(0.5), Some([1.0, 0.0, 0.0]));
        assert_eq!(l.eval(f64::NAN), None);
    }

    #[test]
    fn nothing_to_export_is_an_error() {
        assert!(matches!(usd_export_bodies(&[], &StepHeader::default()), Err(KernelError::Invalid(_))));
    }

    /// With `USD_OUT_DIR` set, write example files for checking against NVIDIA's validator.
    #[test]
    fn write_examples_when_asked() {
        let Ok(dir) = std::env::var("USD_OUT_DIR") else { return };
        let mut cases: Vec<(String, Vec<(&str, Body)>)> = sample_bodies().into_iter().map(|(n, b)| (n.to_string(), vec![(n, b)])).collect();
        cases.push((
            "two_bodies".into(),
            vec![
                ("a", box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap()),
                ("b", cylinder(Vec3::new(50.0, 0.0, 0.0), Vec3::Z, 5.0, 12.0).unwrap()),
            ],
        ));
        for (name, bodies) in &cases {
            let refs: Vec<(&str, &Body)> = bodies.iter().map(|(n, b)| (*n, b)).collect();
            let text = export(&refs);
            check_all(&text);
            std::fs::write(format!("{dir}/{name}.usda"), text).unwrap();
            // The kernel's own volume and area of each body, for comparing with other readers.
            let sizes: Vec<String> =
                bodies.iter().map(|(_, b)| crate::measure(b).map(|m| format!("{} {}", m.volume, m.area)).unwrap_or_default()).collect();
            std::fs::write(format!("{dir}/{name}.vol"), sizes.join("\n")).unwrap();
        }
        let (a, b) = (box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap(), cylinder(Vec3::new(50.0, 0.0, 0.0), Vec3::Z, 5.0, 12.0).unwrap());
        let bodies = [
            ExportBody { name: "Bracket left 1".into(), body: &a, color: Some([1.0, 0.5, 0.0]) },
            ExportBody { name: "Bracket left 1".into(), body: &b, color: Some([0.2, 0.4, 0.9]) },
        ];
        let text = usd_export_bodies(&bodies, &StepHeader::default()).unwrap();
        check_all(&text);
        std::fs::write(format!("{dir}/named_coloured.usda"), text).unwrap();
    }
}
