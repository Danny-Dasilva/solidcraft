//! The transform a prim and its ancestors give a body, from their `xformOpOrder` lists.
//!
//! Matrices here act on column vectors (`p' = M p`). USD lists the operations outermost first,
//! so `[translate, rotateXYZ, scale]` scales a point first and translates it last: the product is
//! `T * R * S`. A `matrix4d` is written for row vectors, so it is transposed.

use std::collections::HashMap;

use super::super::usda::{Layer, Value, Xform};
use super::{R, V3, dot};

pub(super) type Mat = [[f64; 4]; 4];

const IDENTITY: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

fn mul4(a: &Mat, b: &Mat) -> Mat {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, x) in row.iter_mut().enumerate() {
            *x = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn translation(t: [f64; 3]) -> Mat {
    let mut m = IDENTITY;
    for (row, x) in m.iter_mut().zip(t) {
        row[3] = x;
    }
    m
}

fn scaling(s: [f64; 3]) -> Mat {
    let mut m = IDENTITY;
    for (i, x) in s.into_iter().enumerate() {
        m[i][i] = x;
    }
    m
}

/// Rotation by `deg` degrees about the X, Y or Z axis (0, 1, 2), counter-clockwise.
fn rotation(axis: usize, deg: f64) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut m = IDENTITY;
    m[i][i] = c;
    m[i][j] = -s;
    m[j][i] = s;
    m[j][j] = c;
    m
}

/// The rotation of a quaternion `w + xi + yj + zk`.
fn quaternion(q: [f64; 4]) -> Option<Mat> {
    let n = q.iter().map(|x| x * x).sum::<f64>().sqrt();
    if !(n > 1e-12) {
        return None;
    }
    let [w, x, y, z] = q.map(|c| c / n);
    let mut m = IDENTITY;
    m[0] = [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w), 0.0];
    m[1] = [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w), 0.0];
    m[2] = [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y), 0.0];
    Some(m)
}

/// The inverse of an affine matrix.
fn invert(m: &Mat) -> Option<Mat> {
    let a = |i: usize, j: usize| m[i][j];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if !(det.abs() > 1e-300) {
        return None;
    }
    let mut inv = IDENTITY;
    for (i, row) in inv.iter_mut().take(3).enumerate() {
        for (j, x) in row.iter_mut().take(3).enumerate() {
            // The cofactor of (j, i).
            let (r0, r1) = ((j + 1) % 3, (j + 2) % 3);
            let (c0, c1) = ((i + 1) % 3, (i + 2) % 3);
            *x = (a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)) / det;
        }
    }
    let t = [m[0][3], m[1][3], m[2][3]];
    for row in inv.iter_mut().take(3) {
        row[3] = -(0..3).map(|j| row[j] * t[j]).sum::<f64>();
    }
    inv.iter().flatten().all(|x| x.is_finite()).then_some(inv)
}

fn numbers(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Num(x) => Some(vec![*x]),
        Value::Nums { data, .. } => Some(data.clone()),
        _ => None,
    }
}

/// The matrix of one operation, named `xformOp:<kind>[:<suffix>]`.
fn operation(name: &str, v: &Value) -> R<Mat> {
    let kind = name.strip_prefix("xformOp:").unwrap_or(name).split(':').next().unwrap_or("");
    let n = numbers(v).ok_or_else(|| format!("{name} is not a number"))?;
    if !n.iter().all(|x| x.is_finite()) {
        return Err(format!("{name} is not a finite number"));
    }
    let wrong = || format!("{name} has {} numbers", n.len());
    let m = match kind {
        "translate" => translation(n.as_slice().try_into().map_err(|_| wrong())?),
        "scale" => scaling(n.as_slice().try_into().map_err(|_| wrong())?),
        "rotateX" | "rotateY" | "rotateZ" => {
            let [deg] = n.as_slice() else { return Err(wrong()) };
            rotation(usize::from(kind.ends_with('Y')) + 2 * usize::from(kind.ends_with('Z')), *deg)
        }
        "orient" => quaternion(n.as_slice().try_into().map_err(|_| wrong())?).ok_or_else(|| format!("{name} is not a rotation"))?,
        "transform" => {
            let rows: [f64; 16] = n.as_slice().try_into().map_err(|_| wrong())?;
            let mut m = IDENTITY;
            for (i, row) in m.iter_mut().enumerate() {
                for (j, x) in row.iter_mut().enumerate() {
                    // Row vectors: the file's (row j, column i) is ours (row i, column j).
                    *x = rows[4 * j + i];
                }
            }
            if m[3] != [0.0, 0.0, 0.0, 1.0] {
                return Err(format!("{name} is not an affine matrix"));
            }
            m
        }
        k if k.starts_with("rotate") && k.len() == 9 => {
            // rotateXYZ and the like turn about the first axis named first.
            let letters: Vec<usize> = k[6..].chars().map(|c| "XYZ".find(c).unwrap_or(9)).collect();
            if letters.len() != 3 || letters.iter().any(|a| *a > 2) || n.len() != 3 {
                return Err(wrong());
            }
            letters.iter().fold(IDENTITY, |acc, &axis| mul4(&rotation(axis, n[axis]), &acc))
        }
        other => return Err(format!("the transform operation {other} is not supported")),
    };
    Ok(m)
}

/// The matrix of one prim's own operations, and whether the prim restarts the transform stack
/// instead of continuing its parent's.
fn local(x: &Xform) -> R<(Mat, bool)> {
    let mut local = IDENTITY;
    let mut reset = false;
    for entry in &x.order {
        if entry == "!resetXformStack!" {
            (local, reset) = (IDENTITY, true);
            continue;
        }
        let (invert_it, name) = match entry.strip_prefix("!invert!") {
            Some(n) => (true, n),
            None => (false, entry.as_str()),
        };
        // An operation that is listed but has no plain value (only time samples) is skipped.
        let Some(v) = x.ops.get(name) else { continue };
        let mut m = operation(name, v)?;
        if invert_it {
            m = invert(&m).ok_or_else(|| format!("{name} cannot be inverted"))?;
        }
        local = mul4(&local, &m);
    }
    Ok((local, reset))
}

/// The matrices that place the geometry of a layer's prims in the stage, found once per prim
/// from the prim's parent: many prims under one ancestor cost one product each.
pub(super) struct Worlds<'a> {
    layer: &'a Layer,
    done: HashMap<usize, R<Option<Mat>>>,
}

impl<'a> Worlds<'a> {
    pub(super) fn new(layer: &'a Layer) -> Self {
        Worlds { layer, done: HashMap::new() }
    }

    /// The matrix for the node, or `None` when it is the identity.
    pub(super) fn of(&mut self, node: usize) -> R<Option<Mat>> {
        // The nodes from `node` up to the first one already done.
        let mut todo = Vec::new();
        let mut at = Some(node);
        while let Some(i) = at {
            if self.done.contains_key(&i) {
                break;
            }
            todo.push(i);
            at = self.layer.parent(i);
            if todo.len() > 1024 {
                return Err("prims are nested too deeply".into());
            }
        }
        let mut above: R<Option<Mat>> = at.and_then(|i| self.done.get(&i).cloned()).unwrap_or(Ok(None));
        for &i in todo.iter().rev() {
            let world = match self.layer.xform(i).filter(|x| !x.order.is_empty()) {
                None => above.clone(),
                Some(x) => local(x).and_then(|(m, reset)| {
                    let parent = if reset { None } else { above.clone()? };
                    let w = parent.map_or(m, |p| mul4(&p, &m));
                    if !w.iter().flatten().all(|x| x.is_finite()) {
                        return Err("the transform is not finite".into());
                    }
                    let same = w.iter().zip(&IDENTITY).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-12));
                    Ok((!same).then_some(w))
                }),
            };
            self.done.insert(i, world.clone());
            above = world;
        }
        above
    }
}

/// A transform that keeps shapes: a rotation, a mirror, a uniform scale, a move.
#[derive(Clone, Copy, Debug)]
pub(super) struct Placement {
    /// Rows of the linear part.
    pub l: [[f64; 3]; 3],
    pub t: V3,
    /// Its scale of lengths.
    pub s: f64,
    /// It turns the body inside out (a reflection).
    pub mirror: bool,
}

impl Placement {
    pub(super) const NONE: Placement = Placement { l: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3], s: 1.0, mirror: false };

    pub(super) fn of(m: &Mat) -> R<Placement> {
        let l = [[m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]];
        let col = |j: usize| [l[0][j], l[1][j], l[2][j]];
        let (c0, c1, c2) = (col(0), col(1), col(2));
        let lens = [dot(c0, c0).sqrt(), dot(c1, c1).sqrt(), dot(c2, c2).sqrt()];
        let s = lens[0];
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * s.max(1e-300);
        let square = close(lens[1], s)
            && close(lens[2], s)
            && dot(c0, c1).abs() <= 1e-9 * s * s
            && dot(c0, c2).abs() <= 1e-9 * s * s
            && dot(c1, c2).abs() <= 1e-9 * s * s;
        if !(s > 1e-9 && s < 1e9 && square) {
            return Err("a transform that scales unevenly or shears, or whose scale is zero or huge (only rotations, mirrors, moves and one scale in all directions are applied)".into());
        }
        let cross = [c0[1] * c1[2] - c0[2] * c1[1], c0[2] * c1[0] - c0[0] * c1[2], c0[0] * c1[1] - c0[1] * c1[0]];
        Ok(Placement { l, t: [m[0][3], m[1][3], m[2][3]], s, mirror: dot(cross, c2) < 0.0 })
    }

    /// The point `p` moved (no change of units).
    pub(super) fn point(&self, p: V3) -> V3 {
        let v = self.vector(p);
        [v[0] + self.t[0], v[1] + self.t[1], v[2] + self.t[2]]
    }

    /// The vector `v` turned and scaled.
    pub(super) fn vector(&self, v: V3) -> V3 {
        self.l.map(|row| dot(row, v))
    }
}
