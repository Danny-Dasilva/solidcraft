//! NVIDIA usd-brep (`BrepArray`) import, written from the published schema
//! (`source/schema/omniSolid/resources/schema.usda` of the usd-brep project, Apache-2.0).
//!
//! Every `def BrepArray` prim of a `.usda` file is restated as an equivalent STEP exchange and
//! read by the STEP reader, so USD, IGES and STEP share one geometry and topology path. The
//! prim holds the radial-edge model of one or more breps in flat arrays; this module follows it
//! the way the schema lays it out:
//!
//! - regions, shells and faceuses are packed per brep in order, faces by index, loops in the
//!   order of the faces' indices, edgeuses in the order of the loops; indices are global;
//! - a faceuse is one side of a face. Its side is the surface-normal side when it is `same`.
//!   A solid region's shell therefore lists the faces that bound the material, and a `same`
//!   faceuse there means the normal points into the material, so the STEP face is reversed;
//! - a loop is stored as seen from the normal side: counter-clockwise for the outer loop,
//!   clockwise for inner loops. An edgeuse is `same` when it runs from the edge's first vertex
//!   to its second. A face is written with the surface's normal and those loops; one the shell
//!   needs the other way round is wrapped in ORIENTED_FACE;
//! - the first shell of a solid region is its outer shell and later ones are voids. Each solid
//!   region becomes one body; a shell of a void region whose faces appear on both sides is a
//!   sheet and becomes an open body;
//! - geometry sits in arrays per type. The k-th face of a surface type uses the k-th entry of
//!   that type's arrays (edges likewise), and curves and surfaces follow the STEP conventions
//!   in radians. Vertices and edge directions come from the topology; the edge range only says
//!   which way the curve runs;
//! - a rational B-spline sphere or cone, with its poles or apex as a collapsed row of control
//!   points, is written as the sphere or cone (`fit`): the STEP reader cannot mesh it as a
//!   B-spline.
//!
//! Lengths are scaled to millimetres by the stage's `metersPerUnit`; a file without it is taken
//! to be in millimetres. Edgeuse trim curves, radial links, materials, wire edges and point
//! shells are not read.
//!
//! Files are untrusted: sizes, counts, indices and nesting are bounded, and a prim or body that
//! cannot be read becomes a warning.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::usda::{self, BrepPrim, Value};

mod fit;
mod xform;

/// Largest USD file we read.
pub const MAX_USD_BYTES: usize = 256 << 20;
/// Most STEP entities written.
const MAX_ENTITIES: usize = 4_000_000;
const MAX_WARNINGS: usize = 200;
/// Largest coordinate, in millimetres (the STEP reader's own limit).
const MAX_COORD: f64 = 1e9;
/// Highest B-spline order read (degree 32).
const MAX_ORDER: usize = 33;
/// Most control points of a surface that is tested for being a sphere or a cone.
const MAX_FIT_POINTS: usize = 4096;

type R<T> = Result<T, String>;
type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn unit(a: V3) -> Option<V3> {
    let n = dot(a, a).sqrt();
    (n > 1e-300 && n.is_finite()).then(|| mul(a, 1.0 / n))
}

/// Any direction square to `z`.
fn perp(z: V3) -> V3 {
    let a = if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    unit(sub(a, mul(z, dot(a, z)))).unwrap_or([1.0, 0.0, 0.0])
}

/// A STEP real.
fn real(x: f64) -> String {
    if !x.is_finite() {
        return "0.".into();
    }
    let s = format!("{x:?}");
    match s.split_once('e') {
        Some((m, e)) => {
            let m = if m.contains('.') { m.to_string() } else { format!("{m}.") };
            format!("{m}E{e}")
        }
        None if s.contains('.') => s,
        None => format!("{s}."),
    }
}

/// A STEP string literal (quotes doubled, non-ASCII as `\X2\` UTF-16 runs).
fn quoted(s: &str) -> String {
    let mut o = String::from("'");
    for c in s.chars() {
        match c {
            '\'' => o.push_str("''"),
            '\\' => o.push_str("\\\\"),
            c if c.is_ascii() && !c.is_ascii_control() => o.push(c),
            c if c.is_ascii_control() => {}
            c => {
                o.push_str("\\X2\\");
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(o, "{u:04X}");
                }
                o.push_str("\\X0\\");
            }
        }
    }
    o.push('\'');
    o
}

fn flag(b: bool) -> &'static str {
    if b { ".T." } else { ".F." }
}

/// The STEP exchange being written.
struct Out {
    lines: Vec<String>,
}

impl Out {
    fn add(&mut self, s: impl Into<String>) -> R<u64> {
        if self.lines.len() >= MAX_ENTITIES {
            return Err("the file is too large to import".into());
        }
        self.lines.push(s.into());
        Ok(self.lines.len() as u64)
    }
    fn point(&mut self, p: V3) -> R<u64> {
        self.add(format!("CARTESIAN_POINT('',({},{},{}))", real(p[0]), real(p[1]), real(p[2])))
    }
    fn dir(&mut self, d: V3) -> R<u64> {
        self.add(format!("DIRECTION('',({},{},{}))", real(d[0]), real(d[1]), real(d[2])))
    }
}

/// Warnings, without repeats.
struct Warns(Vec<String>);

impl Warns {
    fn add(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.0.contains(&s) && self.0.len() < MAX_WARNINGS {
            self.0.push(s);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Surf {
    Nurb,
    Plane,
    Cylinder,
    Cone,
    Sphere,
    Torus,
}

impl Surf {
    fn of(token: &str) -> R<Surf> {
        Ok(match token {
            "BrepSurfaceNurbAPI" => Surf::Nurb,
            "BrepSurfacePlaneAPI" => Surf::Plane,
            "BrepSurfaceCylinderAPI" => Surf::Cylinder,
            "BrepSurfaceConeAPI" => Surf::Cone,
            "BrepSurfaceSphereAPI" => Surf::Sphere,
            "BrepSurfaceTorusAPI" => Surf::Torus,
            other => return Err(format!("unknown surface type '{other}'")),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Curve {
    Nurb,
    Line,
    Circle,
    Ellipse,
}

impl Curve {
    fn of(token: &str) -> R<Curve> {
        Ok(match token {
            "BrepCurve3dNurbAPI" => Curve::Nurb,
            "BrepCurve3dLineAPI" => Curve::Line,
            "BrepCurve3dCircleAPI" => Curve::Circle,
            "BrepCurve3dEllipseAPI" => Curve::Ellipse,
            other => return Err(format!("unknown curve type '{other}'")),
        })
    }
}

/// The attributes of one prim, by name. A missing array is an empty one.
struct Src<'a> {
    attrs: &'a HashMap<String, Value>,
}

fn uint(x: f64) -> Option<u32> {
    (x.is_finite() && x >= 0.0 && x.fract() == 0.0 && x <= f64::from(u32::MAX)).then_some(x as u32)
}

fn uints(name: &str, v: &[f64]) -> R<Vec<u32>> {
    v.iter().map(|&x| uint(x).ok_or_else(|| format!("{name} holds {x}, which is not a count or an index"))).collect()
}

impl<'a> Src<'a> {
    fn nums(&self, name: &str, width: usize) -> R<&'a [f64]> {
        match self.attrs.get(name) {
            None => Ok(&[]),
            Some(v) => v
                .nums(width)
                .ok_or_else(|| format!("{name} is not an array of {}", if width == 1 { "numbers".into() } else { format!("{width}-tuples") })),
        }
    }
    fn strs(&self, name: &str) -> R<&'a [String]> {
        match self.attrs.get(name) {
            None => Ok(&[]),
            Some(v) => v.strs().ok_or_else(|| format!("{name} is not an array of tokens")),
        }
    }
    /// An array of numbers however they are grouped: a range may be `double[]` or `double2[]`.
    fn flat(&self, name: &str) -> R<&'a [f64]> {
        match self.attrs.get(name) {
            None => Ok(&[]),
            Some(v) => v.flat().ok_or_else(|| format!("{name} is not an array of numbers")),
        }
    }
    fn uints(&self, name: &str) -> R<Vec<u32>> {
        uints(name, self.nums(name, 1)?)
    }
    /// An array of `n` 3-vectors.
    fn vecs(&self, name: &str, n: usize) -> R<&'a [f64]> {
        let v = self.nums(name, 3)?;
        expect(name, v.len() / 3, n)?;
        Ok(v)
    }
    /// An array of `n` numbers.
    fn scalars(&self, name: &str, n: usize) -> R<&'a [f64]> {
        let v = self.nums(name, 1)?;
        expect(name, v.len(), n)?;
        Ok(v)
    }
}

fn expect(name: &str, got: usize, want: usize) -> R<()> {
    if got == want { Ok(()) } else { Err(format!("{name} has {got} entries where the other arrays need {want}")) }
}

fn total(v: &[u32]) -> R<usize> {
    v.iter().try_fold(0usize, |a, x| a.checked_add(*x as usize)).ok_or_else(|| "the counts are too large".to_string())
}

/// Start offsets of consecutive runs of the given lengths, and one past the last.
fn starts(v: &[u32]) -> Vec<usize> {
    let mut out = Vec::with_capacity(v.len() + 1);
    let mut at = 0usize;
    out.push(at);
    for x in v {
        at = at.saturating_add(*x as usize);
        out.push(at);
    }
    out
}

/// An array that is either absent or has `n` entries.
fn or_zero(name: &str, v: Vec<u32>, n: usize) -> R<Vec<u32>> {
    if v.is_empty() {
        Ok(vec![0; n])
    } else {
        expect(name, v.len(), n)?;
        Ok(v)
    }
}

fn orientation(t: &str) -> R<bool> {
    match t {
        "same" => Ok(true),
        "opposite" => Ok(false),
        other => Err(format!("unknown orientation '{other}'")),
    }
}

/// Packed origin, axis and reference direction of the analytic shapes of one type, with up to
/// two scalars (radii, angle).
#[derive(Default, Clone, Copy)]
struct Ana<'a> {
    origin: &'a [f64],
    axis: &'a [f64],
    refdir: &'a [f64],
    s1: &'a [f64],
    s2: &'a [f64],
}

/// The NURBS of one kind, packed: counts, orders and flat control point, weight and knot arrays
/// with the offset of each entry.
#[derive(Default)]
struct Nurbs<'a> {
    cv: &'a [f64],
    w: &'a [f64],
    uk: &'a [f64],
    vk: &'a [f64],
    uc: Vec<u32>,
    vc: Vec<u32>,
    uo: Vec<u32>,
    vo: Vec<u32>,
    cv_at: Vec<usize>,
    uk_at: Vec<usize>,
    vk_at: Vec<usize>,
}

/// Offsets and total of `f(a[i], b[i])` over a pair of arrays.
fn packed(a: &[u32], b: &[u32], f: impl Fn(usize, usize) -> Option<usize>) -> R<(Vec<usize>, usize)> {
    let mut at = Vec::with_capacity(a.len());
    let mut sum = 0usize;
    for (x, y) in a.iter().zip(b) {
        at.push(sum);
        let n = f(*x as usize, *y as usize).and_then(|n| sum.checked_add(n)).ok_or("the counts are too large")?;
        sum = n;
    }
    Ok((at, sum))
}

/// The validated arrays of one prim.
struct Arrays<'a> {
    regions_per_brep: Vec<u32>,
    region_shells: Vec<u32>,
    region_solid: Vec<bool>,
    shell_faceuses: Vec<u32>,
    shell_wires: Vec<u32>,
    point_shells: usize,
    faceuse_face: Vec<u32>,
    faceuse_same: Vec<bool>,
    face_surf: Vec<Surf>,
    face_loop_at: Vec<usize>,
    loop_eu_at: Vec<usize>,
    eu_edge: Vec<u32>,
    eu_same: Vec<bool>,
    edge_curve: Vec<Curve>,
    edge_range: &'a [f64],
    edge_verts: Vec<u32>,
    vertex: &'a [f64],
    /// For each face, the place of its geometry among the faces of its type (edges likewise).
    surf_at: Vec<usize>,
    curve_at: Vec<usize>,
    planes: Ana<'a>,
    cylinders: Ana<'a>,
    cones: Ana<'a>,
    spheres: Ana<'a>,
    tori: Ana<'a>,
    lines: Ana<'a>,
    circles: Ana<'a>,
    ellipses: Ana<'a>,
    surf_nurbs: Nurbs<'a>,
    curve_nurbs: Nurbs<'a>,
}

fn analytic<'a>(src: &Src<'a>, prefix: &str, origin: &str, axis: &str, refdir: Option<&str>, scalars: &[&str], n: usize) -> R<Ana<'a>> {
    let mut a = Ana { origin: src.vecs(&format!("{prefix}{origin}"), n)?, axis: src.vecs(&format!("{prefix}{axis}"), n)?, ..Ana::default() };
    if let Some(r) = refdir {
        a.refdir = src.vecs(&format!("{prefix}{r}"), n)?;
    }
    if let Some(s) = scalars.first() {
        a.s1 = src.scalars(&format!("{prefix}{s}"), n)?;
    }
    if let Some(s) = scalars.get(1) {
        a.s2 = src.scalars(&format!("{prefix}{s}"), n)?;
    }
    Ok(a)
}

impl<'a> Arrays<'a> {
    fn read(prim: &'a BrepPrim) -> R<Arrays<'a>> {
        let src = Src { attrs: &prim.attrs };
        let regions_per_brep = src.uints("brep:regionCount")?;
        let n_regions = total(&regions_per_brep)?;
        let region_shells = src.uints("region:shellCount")?;
        expect("region:shellCount", region_shells.len(), n_regions)?;
        let region_types = src.strs("region:type")?;
        expect("region:type", region_types.len(), n_regions)?;
        let region_solid = region_types
            .iter()
            .map(|t| match t.as_str() {
                "solidRegion" => Ok(true),
                "voidRegion" => Ok(false),
                other => Err(format!("unknown region type '{other}'")),
            })
            .collect::<R<Vec<bool>>>()?;
        let n_shells = total(&region_shells)?;
        let shell_faceuses = src.uints("shell:faceuseCount")?;
        expect("shell:faceuseCount", shell_faceuses.len(), n_shells)?;
        let shell_wires = or_zero("shell:wireEdgeCount", src.uints("shell:wireEdgeCount")?, n_shells)?;
        let shell_points = src.strs("shell:pointType")?;
        if !shell_points.is_empty() {
            expect("shell:pointType", shell_points.len(), n_shells)?;
        }
        let point_shells = shell_points
            .iter()
            .zip(shell_faceuses.iter().zip(&shell_wires))
            .filter(|(t, (fu, w))| t.as_str() == "BrepPointAPI" && **fu == 0 && **w == 0)
            .count();

        let n_faceuses = total(&shell_faceuses)?;
        let faceuse_face = src.uints("faceuse:faceIndex")?;
        expect("faceuse:faceIndex", faceuse_face.len(), n_faceuses)?;
        let faceuse_orient = src.strs("faceuse:orientationType")?;
        expect("faceuse:orientationType", faceuse_orient.len(), n_faceuses)?;
        let faceuse_same = faceuse_orient.iter().map(|t| orientation(t)).collect::<R<Vec<bool>>>()?;

        let face_types = src.strs("face:surfaceType")?;
        let n_faces = face_types.len();
        let face_surf = face_types.iter().map(|t| Surf::of(t)).collect::<R<Vec<Surf>>>()?;
        let face_loops = src.uints("face:loopCount")?;
        expect("face:loopCount", face_loops.len(), n_faces)?;
        for (i, f) in faceuse_face.iter().enumerate() {
            if *f as usize >= n_faces {
                return Err(format!("faceuse {i} uses face {f}, but there are {n_faces} faces"));
            }
        }
        let n_loops = total(&face_loops)?;
        let loop_eus = src.uints("loop:edgeuseCount")?;
        expect("loop:edgeuseCount", loop_eus.len(), n_loops)?;
        let loop_vertex = src.uints("loop:vertexIndex")?;
        if !loop_vertex.is_empty() {
            expect("loop:vertexIndex", loop_vertex.len(), n_loops)?;
        }
        let n_eus = total(&loop_eus)?;
        let eu_edge = src.uints("edgeuse:edgeIndex")?;
        expect("edgeuse:edgeIndex", eu_edge.len(), n_eus)?;
        let eu_orient = src.strs("edgeuse:orientationType")?;
        expect("edgeuse:orientationType", eu_orient.len(), n_eus)?;
        let eu_same = eu_orient.iter().map(|t| orientation(t)).collect::<R<Vec<bool>>>()?;

        let edge_types = src.strs("edge:curveType")?;
        let n_edges = edge_types.len();
        let edge_curve = edge_types.iter().map(|t| Curve::of(t)).collect::<R<Vec<Curve>>>()?;
        let edge_range = src.flat("edge:range")?;
        expect("edge:range", edge_range.len(), 2 * n_edges)?;
        let edge_verts = uints("edge:vertexIndices", src.flat("edge:vertexIndices")?)?;
        expect("edge:vertexIndices", edge_verts.len(), 2 * n_edges)?;
        for e in &eu_edge {
            if *e as usize >= n_edges {
                return Err(format!("an edgeuse uses edge {e}, but there are {n_edges} edges"));
            }
        }
        let n_vertices = src.strs("vertex:pointType")?.len();
        let vertex = src.vecs("brep:vertexPoint:point:position", n_vertices)?;
        for v in &edge_verts {
            if *v as usize >= n_vertices {
                return Err(format!("an edge uses vertex {v}, but there are {n_vertices} vertices"));
            }
        }
        for (l, n) in loop_eus.iter().enumerate() {
            if let (0, Some(v)) = (*n, loop_vertex.get(l))
                && *v as usize >= n_vertices
            {
                return Err(format!("loop {l} is the vertex {v}, but there are {n_vertices} vertices"));
            }
        }

        let mut counts = [0usize; 6];
        let mut place = |k: usize| -> usize {
            counts.get_mut(k).map_or(0, |c| {
                *c += 1;
                *c - 1
            })
        };
        let surf_at: Vec<usize> = face_surf.iter().map(|s| place(*s as usize)).collect();
        let n_surf = |s: Surf| face_surf.iter().filter(|x| **x == s).count();
        let mut counts = [0usize; 4];
        let mut place = |k: usize| -> usize {
            counts.get_mut(k).map_or(0, |c| {
                *c += 1;
                *c - 1
            })
        };
        let curve_at: Vec<usize> = edge_curve.iter().map(|c| place(*c as usize)).collect();
        let n_curve = |c: Curve| edge_curve.iter().filter(|x| **x == c).count();

        let s = "brep:surface:";
        let planes = analytic(&src, &format!("{s}plane:"), "origin", "axis", Some("refDirection"), &[], n_surf(Surf::Plane))?;
        let cylinders = analytic(&src, &format!("{s}cylinder:"), "origin", "axis", Some("refDirection"), &["radius"], n_surf(Surf::Cylinder))?;
        let cones = analytic(&src, &format!("{s}cone:"), "origin", "axis", Some("refDirection"), &["radius", "semiAngle"], n_surf(Surf::Cone))?;
        let spheres = analytic(&src, &format!("{s}sphere:"), "center", "axis", Some("refDirection"), &["radius"], n_surf(Surf::Sphere))?;
        let tori =
            analytic(&src, &format!("{s}torus:"), "origin", "axis", Some("refDirection"), &["majorRadius", "minorRadius"], n_surf(Surf::Torus))?;
        let lines = analytic(&src, "brep:edge3dLine:curve3d:line:", "origin", "direction", None, &[], n_curve(Curve::Line))?;
        let circles =
            analytic(&src, "brep:edge3dCircle:curve3d:circle:", "center", "axis", Some("refDirection"), &["radius"], n_curve(Curve::Circle))?;
        let ellipses = analytic(
            &src,
            "brep:edge3dEllipse:curve3d:ellipse:",
            "center",
            "axis",
            Some("refDirection"),
            &["xRadius", "yRadius"],
            n_curve(Curve::Ellipse),
        )?;

        let n = n_surf(Surf::Nurb);
        let p = format!("{s}nurb:");
        let (uc, vc) = (src.uints(&format!("{p}uVertexCount"))?, src.uints(&format!("{p}vVertexCount"))?);
        let (uo, vo) = (src.uints(&format!("{p}uOrder"))?, src.uints(&format!("{p}vOrder"))?);
        for (name, v) in [("uVertexCount", &uc), ("vVertexCount", &vc), ("uOrder", &uo), ("vOrder", &vo)] {
            expect(&format!("{p}{name}"), v.len(), n)?;
        }
        let (cv_at, n_cv) = packed(&uc, &vc, usize::checked_mul)?;
        let (uk_at, n_uk) = packed(&uc, &uo, usize::checked_add)?;
        let (vk_at, n_vk) = packed(&vc, &vo, usize::checked_add)?;
        let surf_nurbs = Nurbs {
            cv: src.vecs(&format!("{p}controlVertices"), n_cv)?,
            w: src.scalars(&format!("{p}weights"), n_cv)?,
            uk: src.scalars(&format!("{p}uKnots"), n_uk)?,
            vk: src.scalars(&format!("{p}vKnots"), n_vk)?,
            uc,
            vc,
            uo,
            vo,
            cv_at,
            uk_at,
            vk_at,
        };

        let n = n_curve(Curve::Nurb);
        let p = "brep:edge3dNurb:curve3d:nurb:";
        let (vc, order) = (src.uints(&format!("{p}vertexCount"))?, src.uints(&format!("{p}order"))?);
        expect(&format!("{p}vertexCount"), vc.len(), n)?;
        expect(&format!("{p}order"), order.len(), n)?;
        let (k_at, n_k) = packed(&vc, &order, usize::checked_add)?;
        let n_cv = total(&vc)?;
        let mut cv_at = Vec::with_capacity(n);
        let mut at = 0usize;
        for c in &vc {
            cv_at.push(at);
            at = at.saturating_add(*c as usize);
        }
        let curve_nurbs = Nurbs {
            cv: src.vecs(&format!("{p}controlVertices"), n_cv)?,
            w: src.scalars(&format!("{p}weights"), n_cv)?,
            uk: src.scalars(&format!("{p}knots"), n_k)?,
            vk: &[],
            uc: vc,
            vc: Vec::new(),
            uo: order,
            vo: Vec::new(),
            cv_at,
            uk_at: k_at,
            vk_at: Vec::new(),
        };

        Ok(Arrays {
            regions_per_brep,
            region_shells,
            region_solid,
            shell_faceuses,
            shell_wires,
            point_shells,
            faceuse_face,
            faceuse_same,
            face_surf,
            face_loop_at: starts(&face_loops),
            loop_eu_at: starts(&loop_eus),
            eu_edge,
            eu_same,
            edge_curve,
            edge_range,
            edge_verts,
            vertex,
            surf_at,
            curve_at,
            planes,
            cylinders,
            cones,
            spheres,
            tori,
            lines,
            circles,
            ellipses,
            surf_nurbs,
            curve_nurbs,
        })
    }
}

/// A body to write: a solid region (outer shell, then void shells) or a sheet. Shells are lists
/// of (face, reversed): a reversed face has its normal against the surface's.
enum Part {
    Solid { outer: Vec<(usize, bool)>, voids: Vec<Vec<(usize, bool)>> },
    Sheet(Vec<usize>),
}

/// The bodies of a prim, walking its regions, shells and faceuses.
fn parts(a: &Arrays, w: &mut Warns, label: &str) -> R<Vec<Part>> {
    let (mut region, mut shell, mut fu) = (0usize, 0usize, 0usize);
    let mut out = Vec::new();
    let mut doubled = false;
    for &n_regions in &a.regions_per_brep {
        for _ in 0..n_regions {
            let solid = a.region_solid.get(region).copied().unwrap_or(false);
            let n_shells = a.region_shells.get(region).copied().unwrap_or(0);
            region += 1;
            let mut shells: Vec<Vec<(usize, bool)>> = Vec::new();
            for _ in 0..n_shells {
                let n = a.shell_faceuses.get(shell).copied().unwrap_or(0) as usize;
                shell += 1;
                let mut uses = Vec::with_capacity(n);
                for i in fu..fu.saturating_add(n) {
                    let (Some(f), Some(same)) = (a.faceuse_face.get(i), a.faceuse_same.get(i)) else {
                        return Err("the faceuses are truncated".into());
                    };
                    uses.push((*f as usize, *same));
                }
                fu = fu.saturating_add(n);
                if n == 0 {
                    continue;
                }
                if solid {
                    // The material is on the faceuse's side: a `same` side puts it on the normal's.
                    let mut seen = HashSet::new();
                    let faces: Vec<(usize, bool)> = uses.iter().filter(|(f, _)| seen.insert(*f)).map(|(f, same)| (*f, *same)).collect();
                    doubled |= faces.len() < uses.len();
                    shells.push(faces);
                } else {
                    let mut count: HashMap<usize, u8> = HashMap::new();
                    for (f, _) in &uses {
                        let c = count.entry(*f).or_insert(0);
                        *c = c.saturating_add(1);
                    }
                    let mut seen = HashSet::new();
                    let faces: Vec<usize> =
                        uses.iter().map(|(f, _)| *f).filter(|f| count.get(f).is_some_and(|c| *c >= 2) && seen.insert(*f)).collect();
                    if !faces.is_empty() {
                        out.push(Part::Sheet(faces));
                    }
                }
            }
            let mut it = shells.into_iter();
            if let Some(outer) = it.next() {
                out.push(Part::Solid { outer, voids: it.collect() });
            }
        }
    }
    if doubled {
        w.add(format!("{label}: a face with both sides in a solid's shell is read once"));
    }
    if a.shell_wires.iter().any(|n| *n > 0) {
        w.add(format!("{label}: wire edges are not read"));
    }
    if a.point_shells > 0 {
        w.add(format!("{label}: point shells are not read"));
    }
    Ok(out)
}

/// `(index of first knot, ...)`: a knot vector that never decreases and spans something.
fn knots_ok(k: &[f64]) -> bool {
    k.iter().all(|x| x.is_finite()) && k.windows(2).all(|w| w[1] >= w[0]) && matches!((k.first(), k.last()), (Some(a), Some(b)) if b > a)
}

/// Multiplicities and distinct values of a knot vector, as STEP text.
fn knot_text(knots: &[f64]) -> (String, String) {
    let mut ks: Vec<f64> = Vec::new();
    let mut ms: Vec<usize> = Vec::new();
    for &k in knots {
        match ks.last() {
            Some(&l) if (k - l).abs() <= 1e-14 * (1.0 + k.abs()) => {
                if let Some(m) = ms.last_mut() {
                    *m += 1;
                }
            }
            _ => {
                ks.push(k);
                ms.push(1);
            }
        }
    }
    (ms.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(","), ks.iter().map(|k| real(*k)).collect::<Vec<_>>().join(","))
}

/// A surface written for a face.
#[derive(Clone, Copy)]
struct Made {
    id: u64,
    /// Its normal, in the placed body, is the stored surface normal carried over by the placement.
    /// Always for an analytic surface; for a B-spline the normal turns round when the placement
    /// mirrors; for a recognised sphere or cone it is whichever way the shape's outward normal lies.
    n_rel: bool,
    sphere: bool,
}

/// The checked pieces of one NURBS surface.
struct SurfParts<'n> {
    uc: usize,
    vc: usize,
    uo: usize,
    vo: usize,
    cv0: usize,
    w: &'n [f64],
    uk: &'n [f64],
    vk: &'n [f64],
}

/// A loop without its needles: an edge walked and at once walked back (counting round the loop's
/// end) is a slit into the face and adds nothing to its outline. The schema puts such a slit from
/// a sphere face's outline to a pole inside it, and the STEP reader meshes that face wrongly;
/// without the slit it is right. A loop that is nothing but needles comes back empty.
pub(super) fn without_needles(uses: Vec<(usize, bool)>) -> Vec<(usize, bool)> {
    let cancels = |a: &(usize, bool), b: &(usize, bool)| a.0 == b.0 && a.1 != b.1;
    let mut out: Vec<(usize, bool)> = Vec::with_capacity(uses.len());
    for u in &uses {
        if out.last().is_some_and(|l| cancels(l, u)) {
            out.pop();
        } else {
            out.push(*u);
        }
    }
    let (mut lo, mut hi) = (0, out.len());
    while hi > lo + 1 && matches!((out.get(lo), out.get(hi - 1)), (Some(a), Some(b)) if cancels(a, b)) {
        lo += 1;
        hi -= 1;
    }
    out.get(lo..hi).map(<[_]>::to_vec).unwrap_or_default()
}

/// Writes the STEP entities of one prim's bodies.
struct Tr<'a, 'b> {
    a: &'b Arrays<'a>,
    o: &'b mut Out,
    /// Millimetres per file unit.
    scale: f64,
    /// Where the prim's transform puts the body, in file units.
    place: xform::Placement,
    /// Each face's surface as written.
    surf_ids: Vec<Option<Made>>,
    face_ids: Vec<Option<u64>>,
    curve_ids: Vec<Option<u64>>,
    edge_ids: Vec<Option<u64>>,
    vertex_ids: Vec<Option<u64>>,
}

impl<'a, 'b> Tr<'a, 'b> {
    fn new(a: &'b Arrays<'a>, o: &'b mut Out, scale: f64, place: xform::Placement) -> Self {
        Tr {
            a,
            o,
            scale,
            place,
            surf_ids: vec![None; a.face_surf.len()],
            face_ids: vec![None; a.face_surf.len()],
            curve_ids: vec![None; a.edge_curve.len()],
            edge_ids: vec![None; a.edge_curve.len()],
            vertex_ids: vec![None; a.vertex.len() / 3],
        }
    }

    /// A length in millimetres.
    fn mm(&self, x: f64) -> R<f64> {
        self.coord(x * self.place.s)
    }

    /// A coordinate of the placed body in millimetres.
    fn coord(&self, x: f64) -> R<f64> {
        let y = x * self.scale;
        if y.is_finite() && y.abs() < MAX_COORD { Ok(y) } else { Err(format!("{x} is out of range")) }
    }

    /// A positive length in millimetres (or zero, when `zero_ok`).
    fn radius(&self, s: &[f64], k: usize, zero_ok: bool) -> R<f64> {
        let x = s.get(k).copied().ok_or("a radius is missing")?;
        let r = self.mm(x)?;
        if r > 0.0 || (zero_ok && r == 0.0) { Ok(r) } else { Err(format!("a radius of {x} is not positive")) }
    }

    fn triple(s: &[f64], i: usize) -> R<V3> {
        let k = i.checked_mul(3).ok_or("an index is out of range")?;
        let t = s.get(k..k.checked_add(3).ok_or("an index is out of range")?).ok_or("a vector is missing")?;
        let &[x, y, z] = t else { return Err("a vector is missing".into()) };
        if x.is_finite() && y.is_finite() && z.is_finite() { Ok([x, y, z]) } else { Err("a vector is not a number".into()) }
    }

    /// Point `i` of a packed array of points, placed, in millimetres.
    fn xyz(&self, s: &[f64], i: usize) -> R<V3> {
        let [x, y, z] = self.place.point(Self::triple(s, i)?);
        Ok([self.coord(x)?, self.coord(y)?, self.coord(z)?])
    }

    /// Vector `i` of a packed array of directions, placed.
    fn vec3(&self, s: &[f64], i: usize) -> R<V3> {
        let v = self.place.vector(Self::triple(s, i)?);
        if v.iter().all(|c| c.is_finite()) { Ok(v) } else { Err("a vector is not a number".into()) }
    }

    /// An AXIS2_PLACEMENT_3D at `o` (millimetres) with a unit axis and, as far as possible, `rf`.
    fn placement(&mut self, o: V3, axis: V3, rf: V3) -> R<u64> {
        let z = unit(axis).ok_or("a zero axis")?;
        let x = unit(sub(rf, mul(z, dot(rf, z)))).unwrap_or_else(|| perp(z));
        let (p, d1, d2) = (self.o.point(o)?, self.o.dir(z)?, self.o.dir(x)?);
        self.o.add(format!("AXIS2_PLACEMENT_3D('',#{p},#{d1},#{d2})"))
    }

    /// The placement of entry `k`. A circle or ellipse keeps its sense of turning when the body is
    /// mirrored, so its axis turns the other way.
    fn frame(&mut self, a: &Ana, k: usize, conic: bool) -> R<u64> {
        let o = self.xyz(a.origin, k)?;
        let mut axis = self.vec3(a.axis, k)?;
        if conic && self.place.mirror {
            axis = mul(axis, -1.0);
        }
        let rf = if a.refdir.is_empty() { perp(unit(axis).ok_or("a zero axis")?) } else { self.vec3(a.refdir, k)? };
        self.placement(o, axis, rf)
    }

    /// The surface of face `f` (a NURBS sphere or cone as the shape it is, with the shape's own
    /// outward normal).
    fn surface(&mut self, f: usize) -> R<Made> {
        if let Some(Some(s)) = self.surf_ids.get(f) {
            return Ok(*s);
        }
        let a = self.a;
        let kind = *a.face_surf.get(f).ok_or("a face is out of range")?;
        let k = *a.surf_at.get(f).ok_or("a face is out of range")?;
        let mut opposed = false;
        let mut sphere = kind == Surf::Sphere;
        let id = match kind {
            Surf::Plane => {
                let ax = self.frame(&a.planes, k, false)?;
                self.o.add(format!("PLANE('',#{ax})"))?
            }
            Surf::Cylinder => {
                let (ax, r) = (self.frame(&a.cylinders, k, false)?, self.radius(a.cylinders.s1, k, false)?);
                self.o.add(format!("CYLINDRICAL_SURFACE('',#{ax},{})", real(r)))?
            }
            Surf::Cone => {
                let r = self.radius(a.cones.s1, k, true)?;
                let angle = a
                    .cones
                    .s2
                    .get(k)
                    .copied()
                    .filter(|x| x.is_finite() && x.abs() < std::f64::consts::FRAC_PI_2 - 1e-9)
                    .ok_or("a cone's half angle is not within (-90, 90) degrees")?;
                let ax = self.frame(&a.cones, k, false)?;
                if angle == 0.0 {
                    // The schema reads a zero angle as a cylinder.
                    if r == 0.0 {
                        return Err("a cone with no radius and no angle".into());
                    }
                    self.o.add(format!("CYLINDRICAL_SURFACE('',#{ax},{})", real(r)))?
                } else {
                    self.o.add(format!("CONICAL_SURFACE('',#{ax},{},{})", real(r), real(angle)))?
                }
            }
            Surf::Sphere => {
                let (ax, r) = (self.frame(&a.spheres, k, false)?, self.radius(a.spheres.s1, k, false)?);
                self.o.add(format!("SPHERICAL_SURFACE('',#{ax},{})", real(r)))?
            }
            Surf::Torus => {
                let (ax, big, small) = (self.frame(&a.tori, k, false)?, self.radius(a.tori.s1, k, false)?, self.radius(a.tori.s2, k, false)?);
                self.o.add(format!("TOROIDAL_SURFACE('',#{ax},{},{})", real(big), real(small)))?
            }
            Surf::Nurb => {
                let (id, o, s) = self.nurb_surface(k)?;
                (opposed, sphere) = (o, s);
                id
            }
        };
        // A B-spline carries its own normal, which a mirror turns inside out.
        let n_rel = if kind == Surf::Nurb { self.place.mirror == opposed } else { true };
        let made = Made { id, n_rel, sphere };
        if let Some(slot) = self.surf_ids.get_mut(f) {
            *slot = Some(made);
        }
        Ok(made)
    }

    /// The `count` weights from `at`, all positive.
    fn weights<'n>(n: &'n Nurbs<'_>, at: usize, count: usize) -> R<&'n [f64]> {
        let w = n.w.get(at..at.checked_add(count).ok_or("a count is out of range")?).ok_or("weights are missing")?;
        if w.iter().all(|x| x.is_finite() && *x > 0.0) { Ok(w) } else { Err("a weight is not positive".into()) }
    }

    /// The checked pieces of the k-th NURBS surface.
    fn surf_parts(a: &'b Arrays<'a>, k: usize) -> R<SurfParts<'b>> {
        let n = &a.surf_nurbs;
        let get = |v: &[u32]| v.get(k).map(|x| *x as usize).ok_or("a NURBS surface is missing");
        let (uc, vc, uo, vo) = (get(&n.uc)?, get(&n.vc)?, get(&n.uo)?, get(&n.vo)?);
        if !(2..=MAX_ORDER).contains(&uo) || !(2..=MAX_ORDER).contains(&vo) || uc < uo || vc < vo {
            return Err(format!("a NURBS surface of order {uo} x {vo} with {uc} x {vc} control points"));
        }
        let at = |v: &[usize]| v.get(k).copied().ok_or("a NURBS surface is missing");
        let (cv0, uk0, vk0) = (at(&n.cv_at)?, at(&n.uk_at)?, at(&n.vk_at)?);
        let count = uc.checked_mul(vc).ok_or("a NURBS surface is too large")?;
        let w = Self::weights(n, cv0, count)?;
        let uk = n.uk.get(uk0..uk0 + uc + uo).ok_or("knots are missing")?;
        let vk = n.vk.get(vk0..vk0 + vc + vo).ok_or("knots are missing")?;
        if !knots_ok(uk) || !knots_ok(vk) {
            return Err("a NURBS surface has a bad knot vector".into());
        }
        Ok(SurfParts { uc, vc, uo, vo, cv0, w, uk, vk })
    }

    /// The k-th NURBS surface: a sphere or cone it is exactly, else the B-spline. Whether the
    /// normal written is against the stored one, and whether it is a sphere.
    fn nurb_surface(&mut self, k: usize) -> R<(u64, bool, bool)> {
        let a = self.a;
        let n = Self::surf_parts(a, k)?;
        let (uc, vc, uo, vo, cv0, w, uk, vk) = (n.uc, n.vc, n.uo, n.vo, n.cv0, n.w, n.uk, n.vk);
        if uc * vc <= MAX_FIT_POINTS {
            let mut cp = Vec::with_capacity(uc * vc);
            for (i, wi) in w.iter().enumerate() {
                let p = self.xyz(a.surf_nurbs.cv, cv0 + i)?;
                cp.push([p[0] * wi, p[1] * wi, p[2] * wi, *wi]);
            }
            let patch = fit::Patch { uc, vc, p: uo - 1, q: vo - 1, uk, vk, cp };
            if let Some((shape, opposed)) = fit::recognize(&patch) {
                let sphere = matches!(shape, fit::Shape::Sphere { .. });
                let id = match shape {
                    fit::Shape::Sphere { center, radius, axis } => {
                        let ax = self.placement(center, axis, perp(axis))?;
                        self.o.add(format!("SPHERICAL_SURFACE('',#{ax},{})", real(radius)))?
                    }
                    fit::Shape::Cone { apex, axis, angle } => {
                        let ax = self.placement(apex, axis, perp(axis))?;
                        self.o.add(format!("CONICAL_SURFACE('',#{ax},0.,{})", real(angle)))?
                    }
                };
                return Ok((id, opposed, sphere));
            }
        }
        let rational = w.iter().any(|x| (x - 1.0).abs() > 1e-12);
        let mut rows = Vec::with_capacity(uc);
        let mut wrows = Vec::with_capacity(uc);
        for i in 0..uc {
            let mut row = Vec::with_capacity(vc);
            let mut wrow = Vec::with_capacity(vc);
            for j in 0..vc {
                let p = self.xyz(a.surf_nurbs.cv, cv0 + i * vc + j)?;
                row.push(format!("#{}", self.o.point(p)?));
                wrow.push(real(w.get(i * vc + j).copied().unwrap_or(1.0)));
            }
            rows.push(format!("({})", row.join(",")));
            wrows.push(format!("({})", wrow.join(",")));
        }
        let (ms, ks) = knot_text(uk);
        let (mt, kt) = knot_text(vk);
        let (du, dv) = (uo - 1, vo - 1);
        let id = if rational {
            self.o.add(format!(
                "(BOUNDED_SURFACE()B_SPLINE_SURFACE({du},{dv},({}),.UNSPECIFIED.,.U.,.U.,.U.)B_SPLINE_SURFACE_WITH_KNOTS(({ms}),({mt}),({ks}),({kt}),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(({}))REPRESENTATION_ITEM('')SURFACE())",
                rows.join(","),
                wrows.join(",")
            ))?
        } else {
            self.o.add(format!(
                "B_SPLINE_SURFACE_WITH_KNOTS('',{du},{dv},({}),.UNSPECIFIED.,.U.,.U.,.U.,({ms}),({mt}),({ks}),({kt}),.UNSPECIFIED.)",
                rows.join(",")
            ))?
        };
        Ok((id, false, false))
    }

    /// The curve of edge `e`.
    fn curve(&mut self, e: usize) -> R<u64> {
        if let Some(Some(id)) = self.curve_ids.get(e) {
            return Ok(*id);
        }
        let a = self.a;
        let kind = *a.edge_curve.get(e).ok_or("an edge is out of range")?;
        let k = *a.curve_at.get(e).ok_or("an edge is out of range")?;
        let id = match kind {
            Curve::Line => {
                let o = self.xyz(a.lines.origin, k)?;
                let d = unit(self.vec3(a.lines.axis, k)?).ok_or("a line without a direction")?;
                let (p, dd) = (self.o.point(o)?, self.o.dir(d)?);
                let v = self.o.add(format!("VECTOR('',#{dd},1.)"))?;
                self.o.add(format!("LINE('',#{p},#{v})"))?
            }
            Curve::Circle => {
                let (ax, r) = (self.frame(&a.circles, k, true)?, self.radius(a.circles.s1, k, false)?);
                self.o.add(format!("CIRCLE('',#{ax},{})", real(r)))?
            }
            Curve::Ellipse => {
                let (rx, ry) = (self.radius(a.ellipses.s1, k, false)?, self.radius(a.ellipses.s2, k, false)?);
                if (rx - ry).abs() > 1e-12 * rx.max(ry) {
                    // The major axis direction matters: it must exist.
                    let z = unit(self.vec3(a.ellipses.axis, k)?).ok_or("an ellipse without an axis")?;
                    let rf = self.vec3(a.ellipses.refdir, k)?;
                    if unit(sub(rf, mul(z, dot(rf, z)))).is_none() {
                        return Err("an ellipse without a reference direction".into());
                    }
                }
                let ax = self.frame(&a.ellipses, k, true)?;
                self.o.add(format!("ELLIPSE('',#{ax},{},{})", real(rx), real(ry)))?
            }
            Curve::Nurb => self.nurb_curve(k)?,
        };
        if let Some(slot) = self.curve_ids.get_mut(e) {
            *slot = Some(id);
        }
        Ok(id)
    }

    fn nurb_curve(&mut self, k: usize) -> R<u64> {
        let n = &self.a.curve_nurbs;
        let get = |v: &[u32]| v.get(k).map(|x| *x as usize).ok_or("a NURBS curve is missing");
        let (count, order) = (get(&n.uc)?, get(&n.uo)?);
        if !(2..=MAX_ORDER).contains(&order) || count < order {
            return Err(format!("a NURBS curve of order {order} with {count} control points"));
        }
        let at = |v: &[usize]| v.get(k).copied().ok_or("a NURBS curve is missing");
        let (cv0, k0) = (at(&n.cv_at)?, at(&n.uk_at)?);
        let w = Self::weights(n, cv0, count)?;
        let knots = n.uk.get(k0..k0 + count + order).ok_or("knots are missing")?;
        if !knots_ok(knots) {
            return Err("a NURBS curve has a bad knot vector".into());
        }
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let p = self.xyz(self.a.curve_nurbs.cv, cv0 + i)?;
            ids.push(format!("#{}", self.o.point(p)?));
        }
        let (ms, ks) = knot_text(knots);
        let deg = order - 1;
        if w.iter().any(|x| (x - 1.0).abs() > 1e-12) {
            let ws: Vec<String> = w.iter().map(|x| real(*x)).collect();
            self.o.add(format!(
                "(BOUNDED_CURVE()B_SPLINE_CURVE({deg},({}),.UNSPECIFIED.,.U.,.U.)B_SPLINE_CURVE_WITH_KNOTS(({ms}),({ks}),.UNSPECIFIED.)CURVE()GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_CURVE(({}))REPRESENTATION_ITEM(''))",
                ids.join(","),
                ws.join(",")
            ))
        } else {
            self.o.add(format!("B_SPLINE_CURVE_WITH_KNOTS('',{deg},({}),.UNSPECIFIED.,.U.,.U.,({ms}),({ks}),.UNSPECIFIED.)", ids.join(",")))
        }
    }

    fn vertex(&mut self, v: usize) -> R<u64> {
        if let Some(Some(id)) = self.vertex_ids.get(v) {
            return Ok(*id);
        }
        let p = self.xyz(self.a.vertex, v)?;
        let c = self.o.point(p)?;
        let id = self.o.add(format!("VERTEX_POINT('',#{c})"))?;
        if let Some(slot) = self.vertex_ids.get_mut(v) {
            *slot = Some(id);
        }
        Ok(id)
    }

    fn edge(&mut self, e: usize) -> R<u64> {
        if let Some(Some(id)) = self.edge_ids.get(e) {
            return Ok(*id);
        }
        let a = self.a;
        let (Some(&v0), Some(&v1)) = (a.edge_verts.get(2 * e), a.edge_verts.get(2 * e + 1)) else { return Err("an edge is out of range".into()) };
        let (Some(&r0), Some(&r1)) = (a.edge_range.get(2 * e), a.edge_range.get(2 * e + 1)) else { return Err("an edge is out of range".into()) };
        if !(r0.is_finite() && r1.is_finite()) {
            return Err(format!("edge {e} has a range that is not a number"));
        }
        let (vs, ve) = (self.vertex(v0 as usize)?, self.vertex(v1 as usize)?);
        let curve = self.curve(e)?;
        let id = self.o.add(format!("EDGE_CURVE('',#{vs},#{ve},#{curve},{})", flag(r1 >= r0)))?;
        if let Some(slot) = self.edge_ids.get_mut(e) {
            *slot = Some(id);
        }
        Ok(id)
    }

    /// Face `f` as an ADVANCED_FACE with the surface's own normal, its loops counter-clockwise
    /// about that normal (the stored loops, reversed for a shape of the other handedness).
    fn face(&mut self, f: usize) -> R<u64> {
        if let Some(Some(id)) = self.face_ids.get(f) {
            return Ok(*id);
        }
        let a = self.a;
        let Made { id: surface, n_rel, sphere } = self.surface(f)?;
        let (Some(&l0), Some(&l1)) = (a.face_loop_at.get(f), a.face_loop_at.get(f + 1)) else { return Err("a face is out of range".into()) };
        let mut loops: Vec<Vec<(usize, bool)>> = Vec::new();
        for l in l0..l1 {
            let (Some(&u0), Some(&u1)) = (a.loop_eu_at.get(l), a.loop_eu_at.get(l + 1)) else { return Err("a loop is out of range".into()) };
            if u0 == u1 {
                // A loop of one vertex bounds nothing a solid needs.
                continue;
            }
            let mut uses: Vec<(usize, bool)> = Vec::with_capacity(u1 - u0);
            for u in u0..u1 {
                let (Some(&e), Some(&same)) = (a.eu_edge.get(u), a.eu_same.get(u)) else { return Err("an edgeuse is out of range".into()) };
                uses.push((e as usize, same));
            }
            loops.push(uses);
        }
        if sphere {
            // A loop that is only a slit (the schema's closed outline of a sphere face whose real
            // outline is an inner loop) gives way to the next: that one has the face on its left.
            let cut: Vec<Vec<(usize, bool)>> = loops.iter().map(|l| without_needles(l.clone())).collect();
            if cut.iter().any(|l| !l.is_empty()) {
                loops = cut.into_iter().filter(|l| !l.is_empty()).collect();
            }
        }
        let mut bounds: Vec<String> = Vec::new();
        // The stored loops turn counter-clockwise about the stored normal; the surface written
        // needs them so about its own, which a mirror or a shape of the other handedness reverses.
        for mut uses in loops {
            if n_rel == self.place.mirror {
                uses.reverse();
                for u in &mut uses {
                    u.1 = !u.1;
                }
            }
            let mut oriented = Vec::with_capacity(uses.len());
            for (e, forward) in uses {
                let edge = self.edge(e)?;
                oriented.push(format!("#{}", self.o.add(format!("ORIENTED_EDGE('',*,*,#{edge},{})", flag(forward)))?));
            }
            let lp = self.o.add(format!("EDGE_LOOP('',({}))", oriented.join(",")))?;
            let kind = if bounds.is_empty() { "FACE_OUTER_BOUND" } else { "FACE_BOUND" };
            bounds.push(format!("#{}", self.o.add(format!("{kind}('',#{lp},.T.)"))?));
        }
        if bounds.is_empty() {
            return Err(format!("face {f} has no edges"));
        }
        let id = self.o.add(format!("ADVANCED_FACE('',({}),#{surface},.T.)", bounds.join(",")))?;
        if let Some(slot) = self.face_ids.get_mut(f) {
            *slot = Some(id);
        }
        Ok(id)
    }

    /// Face `f` for a shell: `reversed` puts its normal against the stored surface's. A reversed
    /// face is the face turned over by ORIENTED_FACE, which the STEP reader does exactly (a
    /// face with `same_sense` false on a whole sphere it does not mesh).
    fn shell_face(&mut self, f: usize, reversed: bool) -> R<u64> {
        let id = self.face(f)?;
        let n_rel = self.surface(f)?.n_rel;
        if reversed == n_rel { self.o.add(format!("ORIENTED_FACE('',*,#{id},.F.)")) } else { Ok(id) }
    }

    fn shell(&mut self, faces: &[(usize, bool)], closed: bool) -> R<u64> {
        let mut ids = Vec::with_capacity(faces.len());
        for &(f, reversed) in faces {
            ids.push(format!("#{}", self.shell_face(f, reversed)?));
        }
        self.o.add(format!("{}('',({}))", if closed { "CLOSED_SHELL" } else { "OPEN_SHELL" }, ids.join(",")))
    }

    /// The representation item of a body.
    fn part(&mut self, name: &str, part: Part) -> R<u64> {
        let name = quoted(name);
        match part {
            Part::Solid { outer, voids } => {
                let s = self.shell(&outer, true)?;
                if voids.is_empty() {
                    return self.o.add(format!("MANIFOLD_SOLID_BREP({name},#{s})"));
                }
                let mut ids = Vec::with_capacity(voids.len());
                for v in &voids {
                    ids.push(format!("#{}", self.shell(v, true)?));
                }
                self.o.add(format!("BREP_WITH_VOIDS({name},#{s},({}))", ids.join(",")))
            }
            Part::Sheet(faces) => {
                let faces: Vec<(usize, bool)> = faces.into_iter().map(|f| (f, false)).collect();
                let s = self.shell(&faces, false)?;
                self.o.add(format!("SHELL_BASED_SURFACE_MODEL({name},(#{s}))"))
            }
        }
    }
}

/// USD text restated as a STEP exchange (millimetres), with what could not be read.
pub fn usd_to_step(text: &str) -> std::result::Result<(String, Vec<String>), String> {
    crate::guard("USD import", || translate(text).map_err(crate::KernelError::Invalid)).map_err(|e| match e {
        crate::KernelError::Invalid(m) => m,
        other => other.to_string(),
    })
}

/// The colour of a prim's bodies: the first `primvars:displayColor` value when it is constant
/// (USD's default).
fn prim_color(prim: &BrepPrim) -> Option<[f32; 3]> {
    let values = prim.attrs.get("primvars:displayColor")?.nums(3)?;
    if prim.attrs.get("primvars:displayColor:interpolation").and_then(Value::as_str).is_some_and(|i| i != "constant") {
        return None;
    }
    let &[r, g, b, ..] = values else { return None };
    (r.is_finite() && g.is_finite() && b.is_finite()).then(|| [r, g, b].map(|c| c.clamp(0.0, 1.0) as f32))
}

pub(super) fn translate(text: &str) -> R<(String, Vec<String>)> {
    if text.len() > MAX_USD_BYTES {
        return Err("USD file too large".into());
    }
    let layer = usda::parse(text)?;
    if layer.breps.is_empty() {
        return Err("USD import: the file has no BrepArray prim".into());
    }
    let mut w = Warns(Vec::new());
    // Millimetres per file unit: a file that does not say is in millimetres.
    let scale = match layer.meters_per_unit {
        None => 1.0,
        Some(m) if m.is_finite() && m > 0.0 && m <= 1e6 => m * 1000.0,
        Some(m) => return Err(format!("USD import: metersPerUnit {m} is not a usable unit")),
    };
    if layer.up_axis.as_deref().is_some_and(|u| u.eq_ignore_ascii_case("y")) {
        w.add("the file is Y-up; its coordinates are used as written");
    }
    let mut out = Out { lines: Vec::new() };
    let mm = out.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))")?;
    let rad = out.add("(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))")?;
    let sr = out.add("(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT())")?;
    // The vertex tolerance: the files' own, else a small default.
    let tol = layer
        .breps
        .iter()
        .filter_map(|p| p.attrs.get("brep:intersectTol3d")?.nums(1))
        .flatten()
        .filter(|t| t.is_finite() && **t > 0.0)
        .fold(0.0f64, |m, t| m.max(*t))
        * scale;
    let tol = if tol > 0.0 { tol.clamp(1e-6, 0.1) } else { 1e-4 };
    let unc = out.add(format!("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({}),#{mm},'distance_accuracy_value','')", real(tol)))?;
    let ctx = out.add(format!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{unc}))GLOBAL_UNIT_ASSIGNED_CONTEXT((#{mm},#{rad},#{sr}))REPRESENTATION_CONTEXT('',''))"
    ))?;

    let mut items: Vec<u64> = Vec::new();
    let mut styles: Vec<(u64, [f32; 3])> = Vec::new();
    let mut worlds = xform::Worlds::new(&layer);
    for prim in &layer.breps {
        let path = layer.path(prim.node);
        let label = path.as_str();
        // The transform of the prim and its ancestors. One that is malformed leaves the body where
        // the file puts its numbers; one that would change shapes is not read yet.
        if layer.animated(prim.node) {
            w.add(format!("{label}: a transform with time samples is not followed"));
        }
        let place = match worlds.of(prim.node) {
            Ok(None) => xform::Placement::NONE,
            Ok(Some(m)) => match xform::Placement::of(&m) {
                Ok(p) => p,
                Err(e) => {
                    w.add(format!("{label}: not read: not supported yet: {e}"));
                    continue;
                }
            },
            Err(e) => {
                w.add(format!("{label}: its transform is not applied ({e}); the model is placed as written"));
                xform::Placement::NONE
            }
        };
        let arrays = match Arrays::read(prim) {
            Ok(a) => a,
            Err(e) => {
                w.add(format!("{label}: not read: {e}"));
                continue;
            }
        };
        let list = match parts(&arrays, &mut w, label) {
            Ok(p) => p,
            Err(e) => {
                w.add(format!("{label}: not read: {e}"));
                continue;
            }
        };
        let n = list.len();
        // A name of SolidCraft's own in `customData`, else the prim's; a prim of several bodies
        // numbers them.
        let base = prim.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or(&prim.name);
        let color = prim_color(prim);
        let mut tr = Tr::new(&arrays, &mut out, scale, place);
        for (k, part) in list.into_iter().enumerate() {
            let name = if n == 1 { base.to_string() } else { format!("{base}_{}", k + 1) };
            match tr.part(&name, part) {
                Ok(id) => {
                    items.push(id);
                    if let Some(c) = color {
                        styles.push((id, c));
                    }
                }
                Err(e) => w.add(format!("{label}: {name}: not read: {e}")),
            }
        }
    }
    if items.is_empty() {
        let why = w.0.iter().find(|m| m.contains("not read")).or(w.0.first()).cloned().unwrap_or_else(|| "no solids".into());
        return Err(format!("USD import: nothing to read ({why})"));
    }
    let list = items.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(",");
    out.add(format!("ADVANCED_BREP_SHAPE_REPRESENTATION('',({list}),#{ctx})"))?;
    for (item, c) in styles {
        let rgb = out.add(format!("COLOUR_RGB('',{},{},{})", real(f64::from(c[0])), real(f64::from(c[1])), real(f64::from(c[2]))))?;
        let fasc = out.add(format!("FILL_AREA_STYLE_COLOUR('',#{rgb})"))?;
        let fas = out.add(format!("FILL_AREA_STYLE('',(#{fasc}))"))?;
        let ssfa = out.add(format!("SURFACE_STYLE_FILL_AREA(#{fas})"))?;
        let sss = out.add(format!("SURFACE_SIDE_STYLE('',(#{ssfa}))"))?;
        let ssu = out.add(format!("SURFACE_STYLE_USAGE(.BOTH.,#{sss})"))?;
        let psa = out.add(format!("PRESENTATION_STYLE_ASSIGNMENT((#{ssu}))"))?;
        out.add(format!("STYLED_ITEM('color',(#{psa}),#{item})"))?;
    }
    let mut s = String::from(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('USD import'),'2;1');\nFILE_NAME('','',(''),(''),'SolidCraft USD','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n",
    );
    for (i, l) in out.lines.iter().enumerate() {
        let _ = writeln!(s, "#{}={l};", i + 1);
    }
    s.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    Ok((s, w.0))
}
