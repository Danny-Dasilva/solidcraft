//! Spheres and cones written as rational B-splines.
//!
//! A NURBS sphere or cone has its poles, or its apex, as a boundary row of control points that
//! all sit on one point. The STEP reader meshes such a surface wrongly (a cone comes out at twice
//! its volume), but reads the same shape as a sphere or cone without trouble. So a patch with a
//! collapsed boundary is sampled, and when the samples lie on a sphere or a cone to within 1e-8 of
//! the patch's size it is restated as that surface. Anything else stays a B-spline.

use super::{V3, dot, mul, sub, unit};

/// A rational B-spline surface in homogeneous coordinates (`x*w, y*w, z*w, w`), U-major.
pub(super) struct Patch<'a> {
    pub uc: usize,
    pub vc: usize,
    /// Degrees.
    pub p: usize,
    pub q: usize,
    pub uk: &'a [f64],
    pub vk: &'a [f64],
    pub cp: Vec<[f64; 4]>,
}

/// What a patch turned out to be.
pub(super) enum Shape {
    Sphere {
        center: V3,
        radius: f64,
        axis: V3,
    },
    /// Radius zero at `apex`, opening along `axis`.
    Cone {
        apex: V3,
        axis: V3,
        angle: f64,
    },
}

/// The knot span of `u`: the `s` in `p..n` with `k[s] <= u < k[s+1]` (the last non-empty span at
/// the top of the domain).
fn span(k: &[f64], n: usize, p: usize, u: f64) -> Option<usize> {
    let (lo, hi) = (*k.get(p)?, *k.get(n)?);
    if u >= hi {
        let mut s = n.checked_sub(1)?;
        while s > p && k.get(s)? >= k.get(s + 1)? {
            s -= 1;
        }
        return Some(s);
    }
    if u <= lo {
        return Some(p);
    }
    let (mut a, mut b) = (p, n);
    while b > a + 1 {
        let mid = (a + b) / 2;
        if u < *k.get(mid)? {
            b = mid;
        } else {
            a = mid;
        }
    }
    Some(a)
}

/// The `p + 1` basis functions that do not vanish on span `s`, at `u`.
fn basis(k: &[f64], s: usize, p: usize, u: f64) -> Option<Vec<f64>> {
    let mut nb = vec![0.0; p + 1];
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];
    *nb.first_mut()? = 1.0;
    for j in 1..=p {
        *left.get_mut(j)? = u - *k.get((s + 1).checked_sub(j)?)?;
        *right.get_mut(j)? = *k.get(s + j)? - u;
        let mut saved = 0.0;
        for r in 0..j {
            let (rr, ll) = (*right.get(r + 1)?, *left.get(j - r)?);
            let den = rr + ll;
            if !(den.abs() > 0.0) {
                return None;
            }
            let temp = *nb.get(r)? / den;
            *nb.get_mut(r)? = saved + rr * temp;
            saved = ll * temp;
        }
        *nb.get_mut(j)? = saved;
    }
    nb.iter().all(|x| x.is_finite()).then_some(nb)
}

impl Patch<'_> {
    /// The parameter box `(u0, u1, v0, v1)`.
    fn domain(&self) -> Option<(f64, f64, f64, f64)> {
        Some((*self.uk.get(self.p)?, *self.uk.get(self.uc)?, *self.vk.get(self.q)?, *self.vk.get(self.vc)?))
    }

    fn point(&self, u: f64, v: f64) -> Option<V3> {
        let (su, sv) = (span(self.uk, self.uc, self.p, u)?, span(self.vk, self.vc, self.q, v)?);
        let (nu, nv) = (basis(self.uk, su, self.p, u)?, basis(self.vk, sv, self.q, v)?);
        let mut h = [0.0f64; 4];
        for (a, na) in nu.iter().enumerate() {
            for (b, nb) in nv.iter().enumerate() {
                let c = self.cp.get((su - self.p + a) * self.vc + (sv - self.q + b))?;
                for (hx, cx) in h.iter_mut().zip(c) {
                    *hx += na * nb * cx;
                }
            }
        }
        let [x, y, z, w] = h;
        (w.abs() > 1e-300).then(|| [x / w, y / w, z / w]).filter(|p| p.iter().all(|c| c.is_finite()))
    }

    /// The surface normal (not unit) at an interior parameter, by central differences.
    fn normal(&self, u: f64, v: f64) -> Option<V3> {
        let (u0, u1, v0, v1) = self.domain()?;
        let (hu, hv) = ((u1 - u0) * 1e-4, (v1 - v0) * 1e-4);
        let su = sub(self.point(u + hu, v)?, self.point(u - hu, v)?);
        let sv = sub(self.point(u, v + hv)?, self.point(u, v - hv)?);
        Some([su[1] * sv[2] - su[2] * sv[1], su[2] * sv[0] - su[0] * sv[2], su[0] * sv[1] - su[1] * sv[0]])
    }

    /// Points of the control net's boundary rows and columns that are all one point.
    fn collapsed(&self, tol: f64) -> Vec<V3> {
        let at = |i: usize, j: usize| -> Option<V3> {
            let c = self.cp.get(i * self.vc + j)?;
            (c[3] > 0.0).then(|| [c[0] / c[3], c[1] / c[3], c[2] / c[3]])
        };
        let mut out: Vec<V3> = Vec::new();
        let (Some(ul), Some(vl)) = (self.uc.checked_sub(1), self.vc.checked_sub(1)) else { return out };
        let lines: [Vec<Option<V3>>; 4] = [
            (0..self.vc).map(|j| at(0, j)).collect(),
            (0..self.vc).map(|j| at(ul, j)).collect(),
            (0..self.uc).map(|i| at(i, 0)).collect(),
            (0..self.uc).map(|i| at(i, vl)).collect(),
        ];
        for line in lines {
            let Some(Some(first)) = line.first().copied() else { continue };
            let same = line.iter().all(|p| p.is_some_and(|p| dist(p, first) <= tol));
            if same && !out.iter().any(|q| dist(*q, first) <= tol) {
                out.push(first);
            }
        }
        out
    }
}

fn dist(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

/// Solve `a x = b` by Gaussian elimination with partial pivoting.
fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for c in 0..N {
        let pivot = (c..N).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if !(a[pivot][c].abs() > 1e-12) {
            return None;
        }
        a.swap(c, pivot);
        b.swap(c, pivot);
        let (row, rhs) = (a[c], b[c]);
        for r in c + 1..N {
            let f = a[r][c] / row[c];
            for (x, p) in a[r][c..].iter_mut().zip(&row[c..]) {
                *x -= f * p;
            }
            b[r] -= f * rhs;
        }
    }
    let mut x = [0.0; N];
    for r in (0..N).rev() {
        let mut s = b[r];
        for k in r + 1..N {
            s -= a[r][k] * x[k];
        }
        x[r] = s / a[r][r];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Least-squares sphere through the points (centre, radius), if they lie on it within `tol`.
fn fit_sphere(pts: &[V3], tol: f64) -> Option<(V3, f64)> {
    let n = pts.len() as f64;
    let g = mul(pts.iter().fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]), 1.0 / n);
    let s = (pts.iter().map(|p| dist(*p, g).powi(2)).sum::<f64>() / n).sqrt();
    if !(s > 0.0) {
        return None;
    }
    // |q|^2 = 2 c.q + d for the points q about g, in units of s.
    let mut ata = [[0.0; 4]; 4];
    let mut atb = [0.0; 4];
    for p in pts {
        let q = mul(sub(*p, g), 1.0 / s);
        let row = [2.0 * q[0], 2.0 * q[1], 2.0 * q[2], 1.0];
        for i in 0..4 {
            for j in 0..4 {
                ata[i][j] += row[i] * row[j];
            }
            atb[i] += row[i] * dot(q, q);
        }
    }
    let x = solve(ata, atb)?;
    let c = [x[0], x[1], x[2]];
    let r2 = x[3] + dot(c, c);
    if !(r2 > 0.0) {
        return None;
    }
    let center = [g[0] + s * c[0], g[1] + s * c[1], g[2] + s * c[2]];
    let radius = s * r2.sqrt();
    pts.iter().all(|p| (dist(*p, center) - radius).abs() <= tol).then_some((center, radius))
}

/// The cone with this apex through the points, if they lie on it within `tol`: its axis (towards
/// the points) and half angle. `size` is the patch's size.
fn fit_cone(pts: &[V3], apex: V3, size: f64, tol: f64) -> Option<(V3, f64)> {
    // Directions w from the apex satisfy w.n = cos(angle): w.m = 1 for m = n / cos(angle).
    let ws: Vec<V3> = pts.iter().filter(|p| dist(**p, apex) > 1e-6 * size).filter_map(|p| unit(sub(*p, apex))).collect();
    if ws.len() < 8 {
        return None;
    }
    let mut ata = [[0.0; 3]; 3];
    let mut atb = [0.0; 3];
    for w in &ws {
        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += w[i] * w[j];
            }
            atb[i] += w[i];
        }
    }
    let m = solve(ata, atb)?;
    let len = dot(m, m).sqrt();
    // cos(angle) = 1 / |m| must be a real angle, neither flat nor a line.
    if !(len > 1.0 + 1e-6) {
        return None;
    }
    let axis = mul(m, 1.0 / len);
    let angle = (1.0 / len).acos();
    if !(angle > 1e-3 && angle < std::f64::consts::FRAC_PI_2 - 1e-3) {
        return None;
    }
    let on_cone = |p: &V3| (dot(sub(*p, apex), axis) - dist(*p, apex) * angle.cos()).abs() <= tol;
    pts.iter().all(on_cone).then_some((axis, angle))
}

/// What the patch is, when it is a sphere or a cone with a collapsed boundary, and whether its
/// normal points against the shape's outward normal.
pub(super) fn recognize(patch: &Patch) -> Option<(Shape, bool)> {
    let (u0, u1, v0, v1) = patch.domain()?;
    if !(u1 > u0 && v1 > v0) {
        return None;
    }
    // The net's size, and its collapsed boundaries (poles, apex).
    let pos: Vec<V3> = patch.cp.iter().filter(|c| c[3] > 0.0).map(|c| [c[0] / c[3], c[1] / c[3], c[2] / c[3]]).collect();
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let size = dist(lo, hi);
    if !(size > 0.0 && size.is_finite()) {
        return None;
    }
    let tol = 1e-8 * size;
    let poles = patch.collapsed(1e-9 * size);
    if poles.is_empty() {
        return None;
    }
    // Samples over the whole domain, ends included.
    let ts: Vec<f64> = [0.0, 1.0].into_iter().chain((0..11).map(|i| (f64::from(i) + 0.5) / 11.0)).collect();
    let mut pts = Vec::with_capacity(ts.len() * ts.len());
    for a in &ts {
        for b in &ts {
            pts.push(patch.point(u0 + (u1 - u0) * a, v0 + (v1 - v0) * b)?);
        }
    }
    // The normals at nine interior places.
    let mut probes = Vec::new();
    for a in [1.0 / 6.0, 0.5, 5.0 / 6.0] {
        for b in [1.0 / 6.0, 0.5, 5.0 / 6.0] {
            let (u, v) = (u0 + (u1 - u0) * a, v0 + (v1 - v0) * b);
            probes.push((patch.point(u, v)?, patch.normal(u, v)?));
        }
    }
    // Whether the patch's normal agrees with `outward` at all probes (or at none).
    let against = |outward: &dyn Fn(V3) -> V3| -> Option<bool> {
        let (mut with, mut counter) = (0, 0);
        for (p, n) in &probes {
            let d = dot(*n, outward(*p));
            let scale = dot(*n, *n).sqrt();
            if d.abs() <= 1e-9 * scale {
                continue;
            }
            if d > 0.0 { with += 1 } else { counter += 1 }
        }
        match (with, counter) {
            (w, 0) if w > 0 => Some(false),
            (0, c) if c > 0 => Some(true),
            _ => None,
        }
    };
    if let Some((center, radius)) = fit_sphere(&pts, tol)
        && radius <= 10.0 * size
    {
        let opposed = against(&|p| sub(p, center))?;
        // The axis through the poles, else any.
        let axis = match poles.as_slice() {
            [a, b, ..] => unit(sub(*b, *a)),
            [a] => unit(sub(*a, center)),
            [] => None,
        }
        .unwrap_or([0.0, 0.0, 1.0]);
        return Some((Shape::Sphere { center, radius, axis }, opposed));
    }
    for apex in poles {
        let Some((axis, angle)) = fit_cone(&pts, apex, size, tol) else { continue };
        let outward = |p: V3| {
            let v = sub(p, apex);
            let rho = sub(v, mul(axis, dot(v, axis)));
            let r = unit(rho).unwrap_or([0.0; 3]);
            sub(mul(r, angle.cos()), mul(axis, angle.sin()))
        };
        let opposed = against(&outward)?;
        return Some((Shape::Cone { apex, axis, angle }, opposed));
    }
    None
}
