use std::f64::consts::PI;

use super::usd_in::translate;
use super::usda;
use super::*;
use crate::{
    Body, BoolOp, ExportBody, ImportedBody, StepHeader, StepImport, boolean, box_solid, cylinder, fillet, measure, sphere, step_export_bodies,
    step_import, torus,
};
use solidcraft_geom::Vec3;

// Test inputs from NVIDIA's usd-brep project (Apache-2.0; see ATTRIBUTION.md).
const CUBE: &str = include_str!("testdata/Cube.usda");
const CUBE_TWO: &str = include_str!("testdata/CubeBrepArray.usda");
const CUBE_VOID: &str = include_str!("testdata/cubeWithVoid.usda");
const CUBE_ROUNDTRIP: &str = include_str!("testdata/Cube_Roundtrip.usda");
const CUBE_MATERIALS: &str = include_str!("testdata/CubeFaceMaterials.usda");
const CUBES_SHARING_A_FACE: &str = include_str!("testdata/nonManifoldCubes.usda");
const CONE: &str = include_str!("testdata/Cone_SmConnect_Brep.usda");
const MOUSE: &str = include_str!("testdata/mouse.usda");
const SPHERE: &str = include_str!("testdata/sphere_nurbs.usda");
// The cube again with planes and lines, and a unit box from the usd-brep library (planes, NURBS edges).
const CUBE_ANALYTIC: &str = include_str!("testdata/analytic_cube.usda");
const LIB_BOX: &str = include_str!("testdata/lib_box.usda");

// Bodies written by the usd-brep library itself (Python package 1.0.0, Apache-2.0), with the volume
// and area the library measures on them and the faces it counts: (what, file, [(volume, area, faces)]).
const LIBRARY: &[(&str, &str, &[(f64, f64, usize)])] = &[
    ("plate with a hole", include_str!("testdata/lib_plate_hole.usda"), &[(11214.924399, 3957.47085, 7)]),
    ("cylinder", include_str!("testdata/lib_cylinder.usda"), &[(942.515748, 534.531445, 3)]),
    ("cone", include_str!("testdata/lib_cone.usda"), &[(150.780763, 174.166018, 2)]),
    ("cone frustum", include_str!("testdata/lib_frustum.usda"), &[(263.977975, 236.813452, 3)]),
    ("rounded box", include_str!("testdata/lib_box_fillet.usda"), &[(7572.640268, 2080.780139, 26)]),
    ("box with a conical hole", include_str!("testdata/lib_box_cone_cut.usda"), &[(17652.028937, 4317.674247, 7)]),
    ("box with a spherical dent", include_str!("testdata/lib_box_sphere_cut.usda"), &[(16928.327743, 4401.351205, 7)]),
    ("box and cylinder", include_str!("testdata/lib_box_cylinder_union.usda"), &[(1141.378123, 662.523395, 9)]),
    ("partial cylinder", include_str!("testdata/lib_partial_cylinder.usda"), &[(691.319877, 488.781581, 5)]),
    ("partial cone", include_str!("testdata/lib_partial_cone.usda"), &[(302.335997, 300.268394, 5)]),
    ("partial sphere", include_str!("testdata/lib_partial_sphere.usda"), &[(1668.686659, 827.142151, 3)]),
    ("partial torus", include_str!("testdata/lib_partial_torus.usda"), &[(1552.249973, 1092.844132, 3)]),
    ("pyramid", include_str!("testdata/lib_pyramid.usda"), &[(400.0, 360.0, 5)]),
    ("pipe", include_str!("testdata/lib_hollow_cylinder.usda"), &[(4024.259154, 2415.136739, 4)]),
    ("box with a spherical cut-out", include_str!("testdata/lib_box_through_sphere.usda"), &[(11040.469821, 5433.385904, 7)]),
    ("rounded cylinder", include_str!("testdata/lib_cylinder_fillet.usda"), &[(2733.371128, 1016.812627, 5)]),
];
// A sphere and a torus, each one face closed on itself, and three bodies in one prim.
const LIB_SPHERE: &str = include_str!("testdata/lib_sphere.usda");
const LIB_TORUS: &str = include_str!("testdata/lib_torus.usda");
const LIB_THREE: &str = include_str!("testdata/lib_two_bodies.usda");

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-9)
}

/// A USD file read the way the application reads it: restated as STEP, then read by the STEP reader.
fn import(text: &str) -> StepImport {
    let (step, warnings) = usd_to_step(text).unwrap();
    let mut imp = step_import(&step).unwrap();
    imp.warnings.splice(0..0, warnings);
    imp
}

/// Volume and area of an imported body.
fn props(b: &ImportedBody) -> (f64, f64) {
    let m = measure(&b.body).unwrap();
    (m.volume, m.area)
}

/// `text` with the first `from` replaced by `to`; the edit must apply.
fn edit(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "no {from:?} to edit");
    text.replacen(from, to, 1)
}

/// The same file with stage metadata.
fn with_stage(text: &str, meta: &str) -> String {
    edit(text, "#usda 1.0\n", &format!("#usda 1.0\n(\n    {meta}\n)\n"))
}

/// The file with transform operations (property lines) on its first `Xform` prim.
fn world_with(text: &str, ops: &str) -> String {
    let at = text.find("def Xform").expect("an Xform prim");
    let open = at + text[at..].find("\n{\n").expect("its body") + 3;
    format!("{}{ops}\n{}", &text[..open], &text[open..])
}

/// The bounding box of an imported body: (min, max).
fn bbox(b: &ImportedBody) -> ([f64; 3], [f64; 3]) {
    let bb = measure(&b.body).unwrap().bbox;
    ([bb.min.x, bb.min.y, bb.min.z], [bb.max.x, bb.max.y, bb.max.z])
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

/// Reading `text` fails with an error (not a panic); the error.
fn rejected(what: &str, text: &str) -> String {
    match std::panic::catch_unwind(|| translate(text)) {
        Ok(Err(e)) => e,
        Ok(Ok(_)) => panic!("{what}: a hostile file was read"),
        Err(_) => panic!("{what}: panicked"),
    }
}

// ---- Fixtures -------------------------------------------------------------------------------

#[test]
fn nurbs_cube_is_a_unit_solid() {
    let imp = import(CUBE);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 1);
    let b = &imp.bodies[0];
    assert_eq!(b.name, "Cube");
    assert!(b.closed);
    assert_eq!(b.body.face_count(), 6);
    let (v, a) = props(b);
    assert!(rel(v, 1.0) < 1e-6 && rel(a, 6.0) < 1e-6, "volume {v}, area {a}");
}

#[test]
fn other_cubes_read_the_same() {
    // Planes and lines, planes with NURBS edges, a typeless root prim, and a cube with materials.
    for (name, text) in [("analytic", CUBE_ANALYTIC), ("library box", LIB_BOX), ("round trip", CUBE_ROUNDTRIP), ("materials", CUBE_MATERIALS)] {
        let imp = import(text);
        assert!(imp.warnings.is_empty(), "{name}: {:?}", imp.warnings);
        assert_eq!(imp.bodies.len(), 1, "{name}");
        let b = &imp.bodies[0];
        assert!(b.closed, "{name}");
        assert_eq!(b.body.face_count(), 6, "{name}");
        let (v, a) = props(b);
        assert!(rel(v, 1.0) < 1e-6 && rel(a, 6.0) < 1e-6, "{name}: volume {v}, area {a}");
    }
}

#[test]
fn edge_ranges_may_be_pairs() {
    let flat = "uniform double[] edge:range = [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]";
    let pairs = format!("uniform double2[] edge:range = [{}]", ["(0, 1)"; 12].join(", "));
    let imp = import(&edit(CUBE, flat, &pairs));
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert!(rel(props(&imp.bodies[0]).0, 1.0) < 1e-6);
}

#[test]
fn two_breps_in_one_prim_make_two_bodies() {
    let imp = import(CUBE_TWO);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    let names: Vec<&str> = imp.bodies.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["brepArray_1", "brepArray_2"]);
    for b in &imp.bodies {
        assert!(b.closed);
        assert_eq!(b.body.face_count(), 6);
        let (v, _) = props(b);
        assert!(rel(v, 1.0) < 1e-6, "volume {v}");
    }
}

#[test]
fn cubes_sharing_a_face_are_two_solids() {
    // Two solid regions in one brep; each keeps its own copy of the shared face, the second
    // with its normal against the surface's.
    let imp = import(CUBES_SHARING_A_FACE);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 2);
    for b in &imp.bodies {
        assert!(b.closed);
        assert_eq!(b.body.face_count(), 6);
        let (v, a) = props(b);
        assert!(rel(v, 1.0) < 1e-6 && rel(a, 6.0) < 1e-6, "volume {v}, area {a}");
    }
}

#[test]
fn solid_with_a_void_has_an_inner_shell() {
    // A unit cube with a sphere of radius 0.2 cut out of its middle; the sphere is a rational
    // B-spline with its poles as collapsed control rows.
    let imp = import(CUBE_VOID);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 1);
    let b = &imp.bodies[0];
    assert!(b.closed);
    assert_eq!(b.body.face_count(), 7);
    assert_eq!(b.body.shell_count(), 2);
    let (v, a) = props(b);
    let ball = 4.0 / 3.0 * PI * 0.2f64.powi(3);
    assert!(rel(v, 1.0 - ball) < 1e-3, "volume {v}");
    assert!(rel(a, 6.0 + 4.0 * PI * 0.04) < 2e-3, "area {a}");
}

#[test]
fn nurbs_cone_is_a_cone() {
    // Radius 3, height 5: the side is a rational B-spline whose apex is a collapsed row.
    let imp = import(CONE);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 1);
    let b = &imp.bodies[0];
    assert!(b.closed);
    assert_eq!(b.file_faces, 2);
    assert_eq!(b.body.face_count(), 2);
    let (v, a) = props(b);
    assert!(rel(v, PI * 9.0 * 5.0 / 3.0) < 1e-3, "volume {v}");
    assert!(rel(a, PI * 3.0 * 34.0f64.sqrt() + PI * 9.0) < 1e-3, "area {a}");
}

#[test]
fn nurbs_sphere_in_two_halves_is_a_sphere() {
    // Two faces on rational B-spline spheres, each trimmed to a half by two meridians; metres.
    let imp = import(SPHERE);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 1);
    let b = &imp.bodies[0];
    assert!(b.closed);
    assert_eq!(b.file_faces, 2);
    let r = 63.5548585652638;
    let (v, a) = props(b);
    assert!(rel(v, 4.0 / 3.0 * PI * r * r * r) < 1e-3, "volume {v}");
    assert!(rel(a, 4.0 * PI * r * r) < 1e-3, "area {a}");
}

#[test]
fn mouse_is_a_solid() {
    let imp = import(MOUSE);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 1);
    let b = &imp.bodies[0];
    assert!(b.closed);
    assert_eq!(b.body.face_count(), 4);
    // The usd-brep project's own coarse tessellation of this part (MouseMesh.usda) encloses 136.5
    // and has an area of 172.6.
    let (v, a) = props(b);
    assert!(rel(v, 136.5) < 0.05, "volume {v}");
    assert!(rel(a, 172.6) < 0.02, "area {a}");
}

#[test]
fn metres_per_unit_scales_to_millimetres() {
    // No unit: millimetres.
    assert_eq!(import(CUBE).bodies.len(), 1);
    for (meta, size) in [("metersPerUnit = 0.001", 1.0f64), ("metersPerUnit = 0.01", 10.0), ("metersPerUnit = 1", 1000.0)] {
        let imp = import(&with_stage(CUBE, meta));
        let (v, a) = props(&imp.bodies[0]);
        assert!(rel(v, size.powi(3)) < 1e-6 && rel(a, 6.0 * size * size) < 1e-6, "{meta}: volume {v}, area {a}");
    }
    // The sphere file says metres: a ball of 0.0636 is 63.6 mm.
    let bbox = measure(&import(SPHERE).bodies[0].body).unwrap().bbox;
    assert!((bbox.max.z - bbox.min.z - 127.1).abs() < 0.1, "height {}", bbox.max.z - bbox.min.z);
}

#[test]
fn y_up_files_warn() {
    let up = with_stage(CUBE, "upAxis = \"Y\"");
    let (_, w) = usd_to_step(&up).unwrap();
    assert!(w.iter().any(|m| m.contains("Y-up")), "{w:?}");
    // An identity transform is no move and no warning.
    let still = world_with(
        CUBE,
        "    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )\n    uniform token[] xformOpOrder = [\"xformOp:transform\"]",
    );
    let (_, w) = usd_to_step(&still).unwrap();
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn sheets_become_open_bodies() {
    // A one-face sheet: both sides of the face are in the same shell of the infinite region.
    let sheet = r#"#usda 1.0
def BrepArray "Sheet" ()
{
    uniform double3[] brep:extent = [(0, 0, 0), (1, 1, 0)]
    uniform uint[] brep:regionCount = [1]
    uniform point3d[] brep:vertexPoint:point:position = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
    uniform token[] vertex:pointType = ["BrepPointAPI", "BrepPointAPI", "BrepPointAPI", "BrepPointAPI"]
    uniform token[] edge:curveType = ["BrepCurve3dLineAPI", "BrepCurve3dLineAPI", "BrepCurve3dLineAPI", "BrepCurve3dLineAPI"]
    uniform double[] edge:range = [0, 1, 0, 1, 0, 1, 0, 1]
    uniform int2[] edge:vertexIndices = [(0, 1), (1, 2), (2, 3), (3, 0)]
    uniform point3d[] brep:edge3dLine:curve3d:line:origin = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
    uniform vector3d[] brep:edge3dLine:curve3d:line:direction = [(1, 0, 0), (0, 1, 0), (-1, 0, 0), (0, -1, 0)]
    uniform token[] face:surfaceType = ["BrepSurfacePlaneAPI"]
    uniform point3d[] brep:surface:plane:origin = [(0, 0, 0)]
    uniform vector3d[] brep:surface:plane:axis = [(0, 0, 1)]
    uniform vector3d[] brep:surface:plane:refDirection = [(1, 0, 0)]
    uniform uint[] face:loopCount = [1]
    uniform uint[] loop:edgeuseCount = [4]
    uniform uint[] edgeuse:edgeIndex = [0, 1, 2, 3]
    uniform token[] edgeuse:orientationType = ["same", "same", "same", "same"]
    uniform uint[] region:shellCount = [1]
    uniform token[] region:type = ["voidRegion"]
    uniform uint[] shell:faceuseCount = [2]
    uniform uint[] faceuse:faceIndex = [0, 0]
    uniform token[] faceuse:orientationType = ["same", "opposite"]
}
"#;
    let imp = import(sheet);
    assert_eq!(imp.bodies.len(), 1, "{:?}", imp.warnings);
    let b = &imp.bodies[0];
    assert!(!b.closed);
    assert_eq!(b.name, "Sheet");
    let (_, a) = props(b);
    assert!(rel(a, 1.0) < 1e-6, "area {a}");
}

#[test]
fn library_bodies_have_the_librarys_volume_and_area() {
    for (what, text, bodies) in LIBRARY {
        let imp = import(text);
        assert!(imp.warnings.is_empty(), "{what}: {:?}", imp.warnings);
        assert_eq!(imp.bodies.len(), bodies.len(), "{what}");
        for (b, (volume, area, faces)) in imp.bodies.iter().zip(*bodies) {
            assert!(b.closed, "{what}");
            assert_eq!(b.file_faces, *faces, "{what}");
            let (v, a) = props(b);
            // The reader's mesh is within a fraction of a percent of the library's exact figures.
            assert!(rel(v, *volume) < 3e-3 && rel(a, *area) < 3e-3, "{what}: volume {v} against {volume}, area {a} against {area}");
        }
    }
}

#[test]
fn several_bodies_in_one_prim_are_read_in_order() {
    let imp = import(LIB_THREE);
    let names: Vec<&str> = imp.bodies.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Brep0_1", "Brep0_2", "Brep0_3"]);
    assert!(imp.bodies.iter().all(|b| b.closed));
    // A box and a cylinder; the ball is the next test's.
    let (v0, a0) = props(&imp.bodies[0]);
    let (v1, a1) = props(&imp.bodies[1]);
    assert!(rel(v0, 6000.0) < 1e-3 && rel(a0, 2200.0) < 1e-3, "box {v0}, {a0}");
    assert!(rel(v1, 942.515748) < 3e-3 && rel(a1, 534.531445) < 3e-3, "cylinder {v1}, {a1}");
}

/// A face that closes on itself in both directions (a whole sphere or torus) is a single face with
/// its seams used twice; the STEP reader does not mesh that yet, so these are the checks it has
/// to pass once it does.
#[test]
#[ignore = "the STEP reader cannot mesh a single-face sphere or torus"]
fn whole_spheres_and_tori_mesh() {
    for (what, text, volume, area) in [("sphere", LIB_SPHERE, 4187.497480, 1255.310433), ("torus", LIB_TORUS, 6327.322347, 3162.946500)] {
        let imp = import(text);
        assert!(imp.warnings.is_empty(), "{what}: {:?}", imp.warnings);
        let (v, a) = props(&imp.bodies[0]);
        assert!(rel(v, volume) < 3e-3 && rel(a, area) < 3e-3, "{what}: volume {v} against {volume}, area {a} against {area}");
    }
    let imp = import(LIB_THREE);
    let (v, a) = props(&imp.bodies[2]);
    assert!(rel(v, 1436.291172) < 3e-3 && rel(a, 615.097785) < 3e-3, "ball {v}, {a}");
}

#[test]
fn needles_are_cut_from_loops() {
    use super::usd_in::without_needles as cut;
    let (a, b, c) = ((0, true), (1, true), (2, true));
    let back = |(i, f): (usize, bool)| (i, !f);
    // Nothing to cut, and a seam between two circles is not a needle.
    assert_eq!(cut(vec![a, b, c]), [a, b, c]);
    assert_eq!(cut(vec![a, b, back(c), back(b)]), [a, b, back(c), back(b)]);
    // A slit: an edge and at once back.
    assert_eq!(cut(vec![a, b, back(b), c]), [a, c]);
    // A slit of two edges, and one that runs round the end of the list.
    assert_eq!(cut(vec![a, b, c, back(c), back(b)]), [a]);
    assert_eq!(cut(vec![back(c), a, b, c]), [a, b]);
    // Nothing but slit goes altogether.
    assert!(cut(vec![back(c), c]).is_empty());
    assert!(cut(Vec::new()).is_empty());
}

/// A rational B-spline is restated as a sphere or a cone only when it is one.
#[test]
fn nurbs_spheres_and_cones_are_restated_only_when_exact() {
    let (step, _) = usd_to_step(CONE).unwrap();
    assert!(step.contains("CONICAL_SURFACE") && !step.contains("RATIONAL_B_SPLINE_SURFACE"));
    let (step, _) = usd_to_step(SPHERE).unwrap();
    assert!(step.contains("SPHERICAL_SURFACE") && !step.contains("RATIONAL_B_SPLINE_SURFACE"));
    let (step, _) = usd_to_step(CUBE_VOID).unwrap();
    assert!(step.contains("SPHERICAL_SURFACE") && !step.contains("RATIONAL_B_SPLINE_SURFACE"));
    // Move one control point of the cone and of the sphere a little: B-splines again.
    let bent = edit(CONE, "(3, 3, 0), (0, 0, 5)", "(3.001, 3, 0), (0, 0, 5)");
    let (step, _) = usd_to_step(&bent).unwrap();
    assert!(step.contains("RATIONAL_B_SPLINE_SURFACE") && !step.contains("CONICAL_SURFACE"));
    let (head, tail) = CUBE_VOID.split_once("brep:surface:nurb:controlVertices").unwrap();
    let bent = format!("{head}brep:surface:nurb:controlVertices{}", edit(tail, "(0.7, 0.5, 0.29999999999999993)", "(0.7, 0.5, 0.2999)"));
    let (step, _) = usd_to_step(&bent).unwrap();
    assert!(step.contains("RATIONAL_B_SPLINE_SURFACE") && !step.contains("SPHERICAL_SURFACE"));
    // A plain B-spline cube face has no collapsed boundary: left alone.
    let (step, _) = usd_to_step(CUBE).unwrap();
    assert!(!step.contains("SPHERICAL_SURFACE") && !step.contains("CONICAL_SURFACE"));
}

// ---- The reader ---------------------------------------------------------------------------

#[test]
fn reader_finds_prims_wherever_they_sit() {
    let text = r##"#usda 1.0
# a comment
(
    doc = """A "quoted" doc # not a comment
    over two lines"""
    metersPerUnit = 0.01
    upAxis = "Z"
    subLayers = [
        @a.usda@, # first
        @b#c.usda@
    ]
    customData = { dictionary nested = { int x = 1 } string s = "}" }
)

def Xform "A" (
    kind = "assembly"
    prepend references = @file.usda@</Ref>
    variants = { string v = "x" }
)
{
    def Scope "B"
    {
        def BrepArray "deep"
        {
            uniform uint[] brep:regionCount = [1, 2,
                3,   # trailing comma follows
            ]
            uniform double3[] brep:extent = [(-.5, 1e-3, 2.), (+3, -4E2, 0)]
            uniform token[] edge:curveType = ["a", 'b']
            uniform double[] edge:range = [inf, -inf, nan]
            double3 xformOp:translate.timeSamples = { 0: (1, 2, 3), 1: (4, 5, 6) }
            uniform token[] xformOpOrder = ["xformOp:translate"]
            custom rel skipped = [</A>, </B>]
            uniform float3[] skipped:also = [(0, 0, 0)] (
                interpolation = "vertex"
            )
            variantSet "v" = {
                "x" { }
            }
        }
    }
    def Mesh "M" { uniform int[] faceVertexCounts = [3, 3] }
    over BrepArray "over" { uniform uint[] brep:regionCount = [9] }
    class BrepArray "template" { uniform uint[] brep:regionCount = [9] }
    def BrepArray "off" ( active = false ) { uniform uint[] brep:regionCount = [9] }
    def Scope "S" ( active = false ) { def BrepArray "inside" { uniform uint[] brep:regionCount = [9] } }
}

def BrepArray "top" { }
"##;
    let layer = usda::parse(text).unwrap();
    assert_eq!(layer.meters_per_unit, Some(0.01));
    assert_eq!(layer.up_axis.as_deref(), Some("Z"));
    let paths: Vec<String> = layer.breps.iter().map(|b| layer.path(b.node)).collect();
    let names: Vec<(&str, &str)> = paths.iter().zip(&layer.breps).map(|(p, b)| (p.as_str(), b.name.as_str())).collect();
    assert_eq!(names, [("/A/B/deep", "deep"), ("/top", "top")]);
    let deep = &layer.breps[0];
    assert_eq!(deep.attrs.get("brep:regionCount").and_then(|v| v.nums(1)), Some(&[1.0, 2.0, 3.0][..]));
    assert_eq!(deep.attrs.get("brep:extent").and_then(|v| v.nums(3)), Some(&[-0.5, 1e-3, 2.0, 3.0, -400.0, 0.0][..]));
    assert_eq!(deep.attrs.get("edge:curveType").and_then(|v| v.strs()), Some(&["a".to_string(), "b".to_string()][..]));
    let range = deep.attrs.get("edge:range").and_then(|v| v.nums(1)).unwrap();
    assert!(range[0] == f64::INFINITY && range[1] == f64::NEG_INFINITY && range[2].is_nan());
    // Only the schema's own attributes are kept.
    assert!(deep.attrs.keys().all(|k| k.starts_with("brep:") || k.starts_with("edge:")), "{:?}", deep.attrs.keys().collect::<Vec<_>>());
    // Its translation has time samples, which are not followed.
    assert!(layer.animated(deep.node));
    assert_eq!(layer.chain(deep.node).len(), 1);
    assert!(layer.breps[1].attrs.is_empty());
}

#[test]
fn reader_takes_either_text_header() {
    for head in ["#usda 1.0", "#sdf 1.4.32", "\u{feff}#usda 1.0"] {
        let layer = usda::parse(&format!("{head}\ndef BrepArray \"b\" {{ }}\n")).unwrap();
        assert_eq!(layer.breps.len(), 1, "{head}");
    }
    for head in ["#usdc 1.0", "#usda", "#usda1.0", "usda 1.0", "#sdfx 1"] {
        assert!(usda::parse(&format!("{head}\ndef BrepArray \"b\" {{ }}\n")).is_err(), "{head}");
    }
}

#[test]
fn reader_keeps_the_transform_operations() {
    let text = r#"#usda 1.0
def Xform "A"
{
    double3 xformOp:translate = (1, 2, 3)
    double3 xformOp:translate:pivot = (4, 5, 6)
    uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:translate:pivot", "!invert!xformOp:translate:pivot"]
    def Xform "NoOrder"
    {
        double3 xformOp:scale = (2, 2, 2)
        def Xform "B"
        {
            double xformOp:rotateZ = 90
            uniform token[] xformOpOrder = ["!resetXformStack!", "xformOp:rotateZ"]
            def BrepArray "b" { }
        }
    }
}
"#;
    let layer = usda::parse(text).unwrap();
    let b = &layer.breps[0];
    // The prim without an order has no effect, so only A and B are kept.
    let chain = layer.chain(b.node);
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].order, ["xformOp:translate", "xformOp:translate:pivot", "!invert!xformOp:translate:pivot"]);
    assert_eq!(chain[0].ops.get("xformOp:translate:pivot").and_then(|v| v.nums(3)), Some(&[4.0, 5.0, 6.0][..]));
    assert_eq!(chain[1].order, ["!resetXformStack!", "xformOp:rotateZ"]);
    assert_eq!(chain[1].ops.get("xformOp:rotateZ").and_then(|v| v.as_f64()), Some(90.0));
    assert!(!layer.animated(b.node));
}

// ---- Hostile files ---------------------------------------------------------------------------

#[test]
fn files_that_are_not_brep_arrays_are_rejected() {
    for (what, text) in [
        ("empty", ""),
        ("blank", "  \n\t\n"),
        ("garbage", "garbage"),
        ("nul", "\u{0}\u{ff}"),
        ("header only", "#usda 1.0\n"),
        ("not a usda header", "#usd 1.0\n"),
        ("no BrepArray", "#usda 1.0\ndef Xform \"x\" { def Mesh \"m\" { } }\n"),
        ("over only", "#usda 1.0\nover BrepArray \"x\" { }\n"),
        ("inactive", "#usda 1.0\ndef BrepArray \"x\" ( active = false ) { }\n"),
        ("binary", "PXR-USDC\u{0}\u{0}\u{0}\u{0}"),
        ("stl", "solid cube\nendsolid cube\n"),
    ] {
        rejected(what, text);
    }
    assert!(rejected("binary", "PXR-USDC\u{0}").contains("binary"));
    assert!(rejected("empty brep array", "#usda 1.0\ndef BrepArray \"x\" { }\n").contains("nothing to read"));
}

#[test]
fn truncated_files_are_rejected() {
    for text in [CUBE, CUBE_VOID, SPHERE] {
        // Every cut that leaves out the closing braces.
        let mut cut = 0;
        loop {
            cut += text.len() / 40 + 1;
            if cut + 8 >= text.len() {
                break;
            }
            let end = (0..=cut).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
            rejected(&format!("cut at {end}"), &text[..end]);
        }
    }
}

#[test]
fn broken_syntax_is_rejected() {
    let unterminated = [
        "#usda 1.0\ndef Xform \"x\" {",
        "#usda 1.0\ndef Xform \"x\" { } }",
        "#usda 1.0\ndef Xform \"x {\n}",
        "#usda 1.0\ndef Xform \"x\" ( doc = \"\"\"never closed ) { }",
        "#usda 1.0\ndef Xform \"x\" ( refs = @never ) { }",
        "#usda 1.0\ndef Xform \"x\" ( refs = </never ) { }",
        "#usda 1.0\ndef Xform \"x\" { uniform int[] a = [1, 2 }",
        "#usda 1.0\ndef Xform \"x\" { uniform int[] a = (1, 2 }",
        "#usda 1.0\ndef Xform \"x\" { uniform int[] a = [1 2 3] x }",
        "#usda 1.0\ndef Xform \"x\" { uniform int[] a = 1.2.3 }",
        "#usda 1.0\ndef Xform \"x\" { uniform int[] a = { 1: 2 }",
        "#usda 1.0\ndef \"\" { }\n)",
        "#usda 1.0\n(",
        "#usda 1.0\n( metersPerUnit = )",
        "#usda 1.0\ndef BrepArray",
        "#usda 1.0\ndef BrepArray \"x\"",
        "#usda 1.0\nfoo bar",
    ];
    for (i, text) in unterminated.iter().enumerate() {
        rejected(&format!("case {i}"), text);
    }
}

#[test]
fn deep_and_huge_syntax_is_rejected() {
    // Nesting that would overflow the stack, and strings that would fill the memory.
    let deep_prims = format!("#usda 1.0\n{}", "def Xform \"a\" {\n".repeat(100_000));
    assert!(rejected("deep prims", &deep_prims).contains("nested too deeply"));
    let deep_lists = format!("#usda 1.0\ndef BrepArray \"a\" {{ uniform int[] brep:regionCount = {} }}", "[".repeat(100_000));
    assert!(rejected("deep lists", &deep_lists).contains("nested too deeply"));
    let deep_tuples = format!("#usda 1.0\ndef BrepArray \"a\" {{ uniform int[] brep:regionCount = {} }}", "(".repeat(100_000));
    assert!(rejected("deep tuples", &deep_tuples).contains("nested too deeply"));
    let deep_dicts = format!("#usda 1.0\n( customData = {} )", "{".repeat(100_000));
    assert!(rejected("deep dictionaries", &deep_dicts).contains("nested too deeply"));
    let long_name = format!("#usda 1.0\ndef Xform \"{}\" {{ }}", "a".repeat(2 << 20));
    assert!(rejected("long name", &long_name).contains("too long"));
    let long_path = format!("#usda 1.0\ndef Xform \"x\" ( refs = <{}> ) {{ }}", "a".repeat(1 << 17));
    rejected("long path", &long_path);
}

/// Hostile numbers and sizes in an otherwise good file.
#[test]
fn bad_arrays_are_rejected() {
    let pos = "uniform point3d[] brep:vertexPoint:point:position = [(1, 1, 1),";
    let w = "uniform double[] brep:surface:nurb:weights = [1,";
    let cases: Vec<(&str, String, &str)> = vec![
        ("NaN coordinate", edit(CUBE, pos, "uniform point3d[] brep:vertexPoint:point:position = [(nan, 1, 1),"), "nothing to read"),
        ("infinite coordinate", edit(CUBE, pos, "uniform point3d[] brep:vertexPoint:point:position = [(-inf, 1, 1),"), "nothing to read"),
        ("overflowing coordinate", edit(CUBE, pos, "uniform point3d[] brep:vertexPoint:point:position = [(1e999, 1, 1),"), "nothing to read"),
        ("huge coordinate", edit(CUBE, pos, "uniform point3d[] brep:vertexPoint:point:position = [(1e12, 1, 1),"), "nothing to read"),
        (
            "NaN control point",
            edit(CUBE, "brep:surface:nurb:controlVertices = [(0, 0, 0),", "brep:surface:nurb:controlVertices = [(nan, 0, 0),"),
            "nothing to read",
        ),
        ("NaN weight", edit(CUBE, w, "uniform double[] brep:surface:nurb:weights = [nan,"), "nothing to read"),
        ("negative weight", edit(CUBE, w, "uniform double[] brep:surface:nurb:weights = [-1,"), "nothing to read"),
        ("zero weight", edit(CUBE, w, "uniform double[] brep:surface:nurb:weights = [0,"), "nothing to read"),
        ("decreasing knots", edit(CUBE, "brep:surface:nurb:uKnots = [0, 0, 1, 1,", "brep:surface:nurb:uKnots = [1, 1, 0, 0,"), "nothing to read"),
        ("NaN knot", edit(CUBE, "brep:surface:nurb:uKnots = [0, 0, 1, 1,", "brep:surface:nurb:uKnots = [nan, 0, 1, 1,"), "nothing to read"),
        ("order zero", edit(CUBE, "brep:surface:nurb:uOrder = [2,", "brep:surface:nurb:uOrder = [0,"), "nothing to read"),
        ("order one", edit(CUBE, "brep:edge3dNurb:curve3d:nurb:order = [2,", "brep:edge3dNurb:curve3d:nurb:order = [1,"), "nothing to read"),
        ("order past the count", edit(CUBE, "brep:surface:nurb:vOrder = [2,", "brep:surface:nurb:vOrder = [9,"), "nothing to read"),
        ("huge order", edit(CUBE, "brep:surface:nurb:vOrder = [2,", "brep:surface:nurb:vOrder = [4294967295,"), "nothing to read"),
        ("huge control point count", edit(CUBE, "brep:surface:nurb:uVertexCount = [2,", "brep:surface:nurb:uVertexCount = [4294967295,"), "entries"),
        ("fractional count", edit(CUBE, "face:loopCount = [1,", "face:loopCount = [1.5,"), "not a count"),
        ("negative count", edit(CUBE, "face:loopCount = [1,", "face:loopCount = [-1,"), "not a count"),
        ("huge region count", edit(CUBE, "brep:regionCount = [2]", "brep:regionCount = [4294967295]"), "entries"),
        ("huge shell count", edit(CUBE, "region:shellCount = [1, 1]", "region:shellCount = [4294967295, 1]"), "entries"),
        ("huge faceuse count", edit(CUBE, "shell:faceuseCount = [6, 6]", "shell:faceuseCount = [4294967295, 6]"), "entries"),
        ("huge loop count", edit(CUBE, "face:loopCount = [1, 1, 1, 1, 1, 1]", "face:loopCount = [4294967295, 1, 1, 1, 1, 1]"), "entries"),
        ("huge edgeuse count", edit(CUBE, "loop:edgeuseCount = [4, 4, 4, 4, 4, 4]", "loop:edgeuseCount = [4294967295, 4, 4, 4, 4, 4]"), "entries"),
        ("counts past u32", edit(CUBE, "brep:regionCount = [2]", "brep:regionCount = [9999999999]"), "not a count"),
        ("face out of range", edit(CUBE, "faceuse:faceIndex = [5, 4,", "faceuse:faceIndex = [5, 40,"), "face 40"),
        ("edge out of range", edit(CUBE, "edgeuse:edgeIndex = [2, 11,", "edgeuse:edgeIndex = [2, 99,"), "edge 99"),
        ("vertex out of range", edit(CUBE, "edge:vertexIndices = [(4, 0),", "edge:vertexIndices = [(4, 80),"), "vertex 80"),
        ("negative vertex", edit(CUBE, "edge:vertexIndices = [(4, 0),", "edge:vertexIndices = [(-1, 0),"), "not a count"),
        ("regions short", edit(CUBE, "region:shellCount = [1, 1]", "region:shellCount = [1]"), "region:shellCount"),
        (
            "region types long",
            edit(CUBE, "region:type = [\"voidRegion\", \"solidRegion\"]", "region:type = [\"voidRegion\", \"solidRegion\", \"voidRegion\"]"),
            "region:type",
        ),
        ("faces short", edit(CUBE, "face:loopCount = [1, 1, 1, 1, 1, 1]", "face:loopCount = [1, 1, 1, 1, 1]"), "face:loopCount"),
        ("edge ranges short", edit(CUBE, "edge:range = [0, 1,", "edge:range = [0,"), "edge:range"),
        ("vertices short", edit(CUBE, pos, "uniform point3d[] brep:vertexPoint:point:position = ["), "brep:vertexPoint:point:position"),
        (
            "NURBS counts short",
            edit(CUBE, "brep:surface:nurb:vVertexCount = [2, 2, 2, 2, 2, 2]", "brep:surface:nurb:vVertexCount = [2, 2, 2, 2, 2]"),
            "vVertexCount",
        ),
        ("weights short", edit(CUBE, "brep:surface:nurb:weights = [1, 1,", "brep:surface:nurb:weights = [1,"), "weights"),
        ("knots long", edit(CUBE, "brep:surface:nurb:vKnots = [0, 0,", "brep:surface:nurb:vKnots = [0, 0, 0,"), "vKnots"),
        ("no control points", edit(CUBE, "brep:surface:nurb:controlVertices", "brep:surface:nurb:controlVertices_"), "controlVertices"),
        ("no surface geometry", edit(CUBE_ANALYTIC, "brep:surface:plane:axis", "brep:surface:plane:axis_"), "plane:axis"),
        (
            "no line geometry",
            edit(CUBE_ANALYTIC, "brep:edge3dLine:curve3d:line:direction", "brep:edge3dLine:curve3d:line:direction_"),
            "line:direction",
        ),
        (
            "tuple of two",
            edit(CUBE, "brep:vertexPoint:point:position = [(1, 1, 1),", "brep:vertexPoint:point:position = [(1, 1),"),
            "brep:vertexPoint:point:position",
        ),
        ("tokens as numbers", edit(CUBE, "face:surfaceType = [\"BrepSurfaceNurbAPI\",", "face:surfaceType = [3,"), "face:surfaceType"),
        ("numbers as tokens", edit(CUBE, "face:loopCount = [1,", "face:loopCount = [\"1\","), "face:loopCount"),
        (
            "unknown surface",
            edit(CUBE, "face:surfaceType = [\"BrepSurfaceNurbAPI\",", "face:surfaceType = [\"BrepSurfaceMystery\","),
            "unknown surface",
        ),
        ("unknown curve", edit(CUBE, "edge:curveType = [\"BrepCurve3dNurbAPI\",", "edge:curveType = [\"BrepCurve3dMystery\","), "unknown curve"),
        ("unknown side", edit(CUBE, "faceuse:orientationType = [\"same\",", "faceuse:orientationType = [\"sideways\","), "unknown orientation"),
        ("unknown region", edit(CUBE, "region:type = [\"voidRegion\",", "region:type = [\"gasRegion\","), "unknown region"),
        ("zero units", with_stage(CUBE, "metersPerUnit = 0"), "metersPerUnit"),
        ("negative units", with_stage(CUBE, "metersPerUnit = -1"), "metersPerUnit"),
        ("NaN units", with_stage(CUBE, "metersPerUnit = nan"), "metersPerUnit"),
        ("infinite units", with_stage(CUBE, "metersPerUnit = inf"), "metersPerUnit"),
        ("huge units", with_stage(CUBE, "metersPerUnit = 1e300"), "metersPerUnit"),
        ("units past the coordinate range", with_stage(CUBE, "metersPerUnit = 1e6"), "nothing to read"),
    ];
    for (what, text, expected) in &cases {
        let e = rejected(what, text);
        assert!(e.contains(expected), "{what}: {e}");
    }
}

/// A body that cannot be read is left out with a warning; the others are kept.
#[test]
fn one_bad_body_does_not_spoil_the_others() {
    for (bad, kept, lost) in [("(1, 1, 0), (3, 1, 1),", "brepArray_1", "brepArray_2"), ("position = [(1, 1, 1),", "brepArray_2", "brepArray_1")] {
        let text = edit(CUBE_TWO, bad, &bad.replacen("(1, 1, 1)", "(nan, 1, 1)", 1).replacen("(3, 1, 1)", "(nan, 1, 1)", 1));
        let (step, warnings) = usd_to_step(&text).unwrap();
        let imp = step_import(&step).unwrap();
        let names: Vec<&str> = imp.bodies.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, [kept], "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("not read") && w.contains(lost)), "{warnings:?}");
    }
}

/// Every cut, scramble and blow-up of a good file ends in a result, never a panic.
#[test]
fn mutated_files_never_panic() {
    let mut cases: Vec<String> = Vec::new();
    let moved = world_with(
        CUBE_ANALYTIC,
        "    double3 xformOp:translate = (1, 2, 3)\n    double3 xformOp:rotateXYZ = (10, 20, 30)\n    double3 xformOp:scale = (2, 2, 2)\n    matrix4d xformOp:transform = ((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (5, 6, 7, 1))\n    quatf xformOp:orient = (1, 0, 0, 0)\n    uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:rotateXYZ\", \"!invert!xformOp:scale\", \"xformOp:transform\", \"xformOp:orient\"]",
    );
    for text in [CUBE_VOID, CUBE_ANALYTIC, CONE, moved.as_str()] {
        // Every digit run changed in turn, a few at a time.
        let digits: Vec<usize> = text.char_indices().filter(|(_, c)| c.is_ascii_digit()).map(|(i, _)| i).collect();
        let step = (digits.len() / 120).max(1);
        for (k, i) in digits.iter().enumerate().step_by(step) {
            for repl in ["9", "-1", "4294967295", "nan", "0", "1e308"] {
                let mut t = String::with_capacity(text.len() + 12);
                t.push_str(&text[..*i]);
                t.push_str(repl);
                t.push_str(&text[*i + 1..]);
                cases.push(t);
            }
            let _ = k;
        }
        // Every comma, bracket and brace dropped in turn, a few at a time.
        let marks: Vec<usize> =
            text.char_indices().filter(|(_, c)| matches!(c, ',' | '[' | ']' | '(' | ')' | '{' | '}' | '=' | '"')).map(|(i, _)| i).collect();
        let step = (marks.len() / 120).max(1);
        for i in marks.iter().step_by(step) {
            cases.push(format!("{}{}", &text[..*i], &text[*i + 1..]));
        }
    }
    for (i, t) in cases.iter().enumerate() {
        let r = std::panic::catch_unwind(|| translate(t));
        assert!(r.is_ok(), "case {i} panicked");
        // The same file through the public entry point, which also reads the STEP it gives.
        if let Ok(Ok((step, _))) = r {
            let r = std::panic::catch_unwind(|| step_import(&step).map(|i| i.bodies.len()));
            assert!(r.is_ok(), "case {i}: the STEP reader panicked");
        }
    }
}

// ---- Round trip -----------------------------------------------------------------------------

/// Export a body to USD and read it back: the same solid as through STEP.
fn round_trip(b: &Body) -> StepImport {
    let body = ExportBody { name: "Bracket left 1".into(), body: b, color: Some([1.0, 0.5, 0.0]) };
    let header = StepHeader { file_name: "t.usda".into(), ..Default::default() };
    let text = usd_export_bodies(&[body], &header).unwrap();
    let imp = import(&text);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 1);
    let u = &imp.bodies[0];
    assert!(u.closed);
    assert_eq!(u.name, "Bracket left 1");
    assert_eq!(u.color, Some([1.0, 0.5, 0.0]));
    let (m0, m1) = (measure(b).unwrap(), measure(&u.body).unwrap());
    assert!(rel(m1.volume, m0.volume) < 1e-3, "volume {} vs {}", m1.volume, m0.volume);
    assert!(rel(m1.area, m0.area) < 1e-3, "area {} vs {}", m1.area, m0.area);
    // The same body through STEP: the same faces.
    let step_text = step_export_bodies(&[ExportBody { name: "S".into(), body: b, color: None }], &StepHeader::default()).unwrap();
    let s = &step_import(&step_text).unwrap().bodies[0];
    assert_eq!(u.body.face_count(), s.body.face_count());
    assert_eq!(u.file_faces, s.file_faces);
    imp
}

#[test]
fn usd_round_trip() {
    round_trip(&box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap());
    round_trip(&cylinder(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 1.0, 1.0), 5.0, 12.0).unwrap());
    round_trip(&sphere(Vec3::new(1.0, 2.0, 3.0), 10.0).unwrap());
    round_trip(&torus(Vec3::ZERO, 20.0, 4.0).unwrap());
    let plate = box_solid(Vec3::ZERO, Vec3::new(40.0, 30.0, 10.0)).unwrap();
    let pin = cylinder(Vec3::new(20.0, 15.0, -1.0), Vec3::Z, 5.0, 12.0).unwrap();
    let holed = boolean(&plate, &pin, BoolOp::Cut).unwrap().unwrap();
    round_trip(&holed);
    round_trip(&fillet(&plate, &[Vec3::new(20.0, 0.0, 10.0)], 3.0).unwrap());
}

#[test]
fn exported_names_and_colours_survive() {
    let a = box_solid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)).unwrap();
    let b = cylinder(Vec3::new(50.0, 0.0, 0.0), Vec3::Z, 5.0, 12.0).unwrap();
    let bodies = [
        ExportBody { name: "Bracket left 1".into(), body: &a, color: Some([1.0, 0.5, 0.0]) },
        ExportBody { name: "Pin".into(), body: &b, color: Some([0.25, 0.5, 0.75]) },
        ExportBody { name: "Plain".into(), body: &a, color: None },
        ExportBody { name: "Bracket left 1".into(), body: &b, color: None },
    ];
    let text = usd_export_bodies(&bodies, &StepHeader::default()).unwrap();
    let imp = import(&text);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    let got: Vec<(&str, Option<[f32; 3]>)> = imp.bodies.iter().map(|b| (b.name.as_str(), b.color)).collect();
    let want: Vec<(&str, Option<[f32; 3]>)> = bodies.iter().map(|b| (b.name.as_str(), b.color)).collect();
    assert_eq!(got, want);
}

const NAMED: &str = r#"#usda 1.0
def Xform "World"
{
    def BrepArray "Cube_prim" (
        customData = {
            string source = ""
            dictionary solidcraft = {
                int ignored = 3
                string name = "Bracket \"left\" 1"
            }
        }
    )
    {
        color3f[] primvars:displayColor = [(1, 0.5, 0), (0, 0, 1)]
        uniform token primvars:displayColor:interpolation = "constant"
    }
}
"#;

#[test]
fn reader_takes_the_name_and_colour_of_a_prim() {
    let prim = |text: &str| {
        let text = format!("{text}\n").replace("        color3f", "        uniform double3[] brep:extent = [(0, 0, 0), (1, 1, 1)]\n        color3f");
        usda::parse(&text).unwrap().breps.remove(0)
    };
    let p = prim(NAMED);
    assert_eq!(p.title.as_deref(), Some("Bracket \"left\" 1"));
    assert_eq!(p.name, "Cube_prim");
    assert_eq!(p.attrs.get("primvars:displayColor").and_then(|v| v.nums(3)), Some(&[1.0, 0.5, 0.0, 0.0, 0.0, 1.0][..]));
    // A flat key, as pxr's GetCustomDataByKey("solidcraft:name") names it, reads the same.
    let flat = NAMED.replace("dictionary solidcraft = {", "").replace("string name =", "string solidcraft:name =").replacen("}\n        }", "}", 1);
    assert_eq!(prim(&flat).title.as_deref(), Some("Bracket \"left\" 1"));
    // Interpolation as metadata on the attribute.
    let meta = NAMED
        .replace("uniform token primvars:displayColor:interpolation = \"constant\"", "")
        .replace("(0, 0, 1)]", "(0, 0, 1)] ( interpolation = \"vertex\" )");
    assert_eq!(prim(&meta).attrs.get("primvars:displayColor:interpolation").and_then(|v| v.as_str()), Some("vertex"));
    // customData that is not a dictionary of strings is skipped, not an error.
    let odd = NAMED.replace(
        "int ignored = 3",
        "double3[] odd = (1, 2, 3)\n                token[] t = [\"a\", \"b\"]\n                dictionary deeper = { string x = \"y\" }",
    );
    assert_eq!(prim(&odd).title.as_deref(), Some("Bracket \"left\" 1"));
}

#[test]
fn bodies_are_named_and_coloured_from_the_prim() {
    // The cube twice in one prim (names numbered), and under a name and a colour of its own.
    let named = edit(
        CUBE,
        "customData = {\n            string source = \"\"\n        }",
        "customData = {\n            dictionary solidcraft = {\n                string name = \"Wing\"\n            }\n        }",
    );
    let coloured = edit(&named, "    )\n    {", "    )\n    {\n        color3f[] primvars:displayColor = [(0.5, 0.25, 1)]");
    let imp = import(&coloured);
    assert_eq!(imp.bodies.len(), 1);
    assert_eq!(imp.bodies[0].name, "Wing");
    assert_eq!(imp.bodies[0].color, Some([0.5, 0.25, 1.0]));
    // Several bodies in one prim: the prim's colour on each, the name numbered.
    let two = edit(
        CUBE_TWO,
        "customData = {\n            string source = \"\"\n        }",
        "customData = {\n            dictionary solidcraft = {\n                string name = \"Pair\"\n            }\n        }",
    );
    let two = edit(&two, "    )\n    {", "    )\n    {\n        color3f[] primvars:displayColor = [(1, 0, 0)]");
    let imp = import(&two);
    let got: Vec<(&str, Option<[f32; 3]>)> = imp.bodies.iter().map(|b| (b.name.as_str(), b.color)).collect();
    assert_eq!(got, [("Pair_1", Some([1.0, 0.0, 0.0])), ("Pair_2", Some([1.0, 0.0, 0.0]))]);
    // Colours that are not constant, or not numbers, are left out.
    let varying = edit(
        &coloured,
        "color3f[] primvars:displayColor = [(0.5, 0.25, 1)]",
        "color3f[] primvars:displayColor = [(0.5, 0.25, 1)] ( interpolation = \"faceVarying\" )",
    );
    assert_eq!(import(&varying).bodies[0].color, None);
    let bad = edit(&coloured, "(0.5, 0.25, 1)", "(nan, 0.25, 1)");
    assert_eq!(import(&bad).bodies[0].color, None);
    // A colour outside 0..1 is clamped.
    let loud = edit(&coloured, "(0.5, 0.25, 1)", "(7, -2, 1)");
    assert_eq!(import(&loud).bodies[0].color, Some([1.0, 0.0, 1.0]));
}

// ---- Transforms ---------------------------------------------------------------------------------

// A plate 40 x 30 x 10 with a round hole (volume 11214.9, area 3957.5), under a `World` prim.
const PLATE: &str = include_str!("testdata/lib_plate_hole.usda");
const PLATE_VOLUME: f64 = 11214.924399;
const PLATE_AREA: f64 = 3957.47085;

/// The plate under `World` operations; its bounding box, volume and area.
fn placed(ops: &str) -> (([f64; 3], [f64; 3]), f64, f64) {
    let imp = import(&world_with(PLATE, ops));
    assert!(imp.warnings.is_empty(), "{ops}: {:?}", imp.warnings);
    assert_eq!(imp.bodies.len(), 1, "{ops}");
    let b = &imp.bodies[0];
    assert!(b.closed, "{ops}");
    let (v, a) = props(b);
    (bbox(b), v, a)
}

/// `ops` on the plate: where its bounding box ends up, and its volume and area against the
/// unplaced plate's, for a length factor (volume x f^3, area x f^2).
fn check(ops: &str, min: [f64; 3], max: [f64; 3], factor: f64) {
    let ((lo, hi), v, a) = placed(ops);
    assert!(near(lo, min) && near(hi, max), "{ops}: box {lo:?} {hi:?}");
    assert!(rel(v, PLATE_VOLUME * factor.powi(3)) < 3e-3, "{ops}: volume {v}");
    assert!(rel(a, PLATE_AREA * factor * factor) < 3e-3, "{ops}: area {a}");
}

fn order(names: &str) -> String {
    format!("    uniform token[] xformOpOrder = [{names}]")
}

#[test]
fn translate_moves_the_body() {
    check(&format!("    double3 xformOp:translate = (1, 2, 3)\n{}", order("\"xformOp:translate\"")), [1.0, 2.0, 3.0], [41.0, 32.0, 13.0], 1.0);
}

#[test]
fn rotate_x_turns_the_body() {
    // (x, y, z) -> (x, -z, y)
    check(&format!("    double xformOp:rotateX = 90\n{}", order("\"xformOp:rotateX\"")), [0.0, -10.0, 0.0], [40.0, 0.0, 30.0], 1.0);
}

#[test]
fn rotate_y_turns_the_body() {
    // (x, y, z) -> (z, y, -x)
    check(&format!("    double xformOp:rotateY = 90\n{}", order("\"xformOp:rotateY\"")), [0.0, 0.0, -40.0], [10.0, 30.0, 0.0], 1.0);
}

#[test]
fn rotate_z_turns_the_body() {
    // (x, y, z) -> (-y, x, z)
    check(&format!("    double xformOp:rotateZ = 90\n{}", order("\"xformOp:rotateZ\"")), [-30.0, 0.0, 0.0], [0.0, 40.0, 10.0], 1.0);
}

#[test]
fn rotate_xyz_turns_about_the_first_axis_first() {
    // X, then Z: (x, y, z) -> (x, -z, y) -> (z, x, y).
    check(&format!("    double3 xformOp:rotateXYZ = (90, 0, 90)\n{}", order("\"xformOp:rotateXYZ\"")), [0.0; 3], [10.0, 40.0, 30.0], 1.0);
    // Z, then X: (x, y, z) -> (-y, x, z) -> (-y, -z, x).
    check(&format!("    double3 xformOp:rotateZYX = (90, 0, 90)\n{}", order("\"xformOp:rotateZYX\"")), [-30.0, -10.0, 0.0], [0.0, 0.0, 40.0], 1.0);
}

#[test]
fn orient_turns_by_a_quaternion() {
    let q = "    quatf xformOp:orient = (0.7071067811865476, 0, 0, 0.7071067811865476)";
    check(&format!("{q}\n{}", order("\"xformOp:orient\"")), [-30.0, 0.0, 0.0], [0.0, 40.0, 10.0], 1.0);
}

#[test]
fn scale_scales_the_body() {
    check(&format!("    double3 xformOp:scale = (2, 2, 2)\n{}", order("\"xformOp:scale\"")), [0.0; 3], [80.0, 60.0, 20.0], 2.0);
}

#[test]
fn a_matrix_is_written_for_row_vectors() {
    // The move is in the last row.
    let m = "    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (5, 6, 7, 1) )";
    check(&format!("{m}\n{}", order("\"xformOp:transform\"")), [5.0, 6.0, 7.0], [45.0, 36.0, 17.0], 1.0);
    // A turn: row 0 is where X goes. X -> Y, Y -> -X: a quarter turn about Z.
    let m = "    matrix4d xformOp:transform = ( (0, 1, 0, 0), (-1, 0, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )";
    check(&format!("{m}\n{}", order("\"xformOp:transform\"")), [-30.0, 0.0, 0.0], [0.0, 40.0, 10.0], 1.0);
}

#[test]
fn the_first_operation_is_the_outermost() {
    // Scale, then turn, then move.
    let ops = format!(
        "    double3 xformOp:translate = (100, 0, 0)\n    double xformOp:rotateZ = 90\n    double3 xformOp:scale = (2, 2, 2)\n{}",
        order("\"xformOp:translate\", \"xformOp:rotateZ\", \"xformOp:scale\"")
    );
    check(&ops, [40.0, 0.0, 0.0], [100.0, 80.0, 20.0], 2.0);
}

#[test]
fn a_pivot_turn_is_move_turn_move_back() {
    let ops = format!(
        "    double3 xformOp:translate:pivot = (20, 15, 0)\n    double xformOp:rotateZ = 90\n{}",
        order("\"xformOp:translate:pivot\", \"xformOp:rotateZ\", \"!invert!xformOp:translate:pivot\"")
    );
    check(&ops, [5.0, -5.0, 0.0], [35.0, 35.0, 10.0], 1.0);
}

#[test]
fn an_inverse_cancels_and_an_unlisted_operation_does_nothing() {
    let ops = format!("    double3 xformOp:translate = (9, 9, 9)\n{}", order("\"xformOp:translate\", \"!invert!xformOp:translate\""));
    check(&ops, [0.0; 3], [40.0, 30.0, 10.0], 1.0);
    check("    double3 xformOp:translate = (9, 9, 9)", [0.0; 3], [40.0, 30.0, 10.0], 1.0);
}

#[test]
fn transforms_of_ancestors_and_prims_combine() {
    // The world moves by 10 in x; the plate scales by 2 about its own corner.
    let text = world_with(PLATE, "    double3 xformOp:translate = (10, 0, 0)\n    uniform token[] xformOpOrder = [\"xformOp:translate\"]");
    let text = edit(
        &text,
        "    {\n        uniform point3d[] brep:edge3dNurb",
        "    {\n        double3 xformOp:scale = (2, 2, 2)\n        uniform token[] xformOpOrder = [\"xformOp:scale\"]\n        uniform point3d[] brep:edge3dNurb",
    );
    let imp = import(&text);
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    let (lo, hi) = bbox(&imp.bodies[0]);
    assert!(near(lo, [10.0, 0.0, 0.0]) && near(hi, [90.0, 60.0, 20.0]), "{lo:?} {hi:?}");
    // A prim that resets the stack ignores its ancestors.
    let reset = edit(
        &text,
        "uniform token[] xformOpOrder = [\"xformOp:scale\"]",
        "uniform token[] xformOpOrder = [\"!resetXformStack!\", \"xformOp:scale\"]",
    );
    let (lo, hi) = bbox(&import(&reset).bodies[0]);
    assert!(near(lo, [0.0, 0.0, 0.0]) && near(hi, [80.0, 60.0, 20.0]), "{lo:?} {hi:?}");
}

#[test]
fn transforms_and_units_combine() {
    // Centimetres: a move of 1 is 10 mm, and the plate is 400 mm long.
    let text = with_stage(
        &world_with(PLATE, "    double3 xformOp:translate = (1, 0, 0)\n    uniform token[] xformOpOrder = [\"xformOp:translate\"]"),
        "metersPerUnit = 0.01",
    );
    let imp = import(&text);
    let (lo, hi) = bbox(&imp.bodies[0]);
    assert!(near(lo, [10.0, 0.0, 0.0]) && near(hi, [410.0, 300.0, 100.0]), "{lo:?} {hi:?}");
}

/// A mirror turns the body over, and it must come out the same solid: the right way round.
#[test]
fn mirrors_give_the_mirrored_solid() {
    let mirrors = [
        "    double3 xformOp:scale = (-1, 1, 1)\n    uniform token[] xformOpOrder = [\"xformOp:scale\"]",
        "    double3 xformOp:scale = (1, 1, -1)\n    double xformOp:rotateZ = 33\n    uniform token[] xformOpOrder = [\"xformOp:rotateZ\", \"xformOp:scale\"]",
        "    double3 xformOp:scale = (-3, -3, 3)\n    double xformOp:rotateY = 20\n    uniform token[] xformOpOrder = [\"xformOp:rotateY\", \"xformOp:scale\"]",
        "    double3 xformOp:scale = (-1, -1, -1)\n    uniform token[] xformOpOrder = [\"xformOp:scale\"]",
    ];
    // Planes with a round hole (analytic), a B-spline cone and two B-spline hemispheres (both
    // recognised), a B-spline solid, and a rounded box (cylinders and tori).
    let bodies = [("plate", PLATE), ("cone", CONE), ("hemispheres", SPHERE), ("mouse", MOUSE), ("rounded box", LIBRARY[4].1)];
    for (what, text) in bodies {
        let plain = import(text);
        let (v0, a0) = props(&plain.bodies[0]);
        for ops in mirrors {
            let k = if ops.contains("-3, -3, 3") { 3.0 } else { 1.0 };
            let imp = import(&world_with(text, ops));
            assert!(imp.warnings.is_empty(), "{what}: {:?}", imp.warnings);
            let b = &imp.bodies[0];
            assert!(b.closed, "{what}");
            assert_eq!(b.body.face_count(), plain.bodies[0].body.face_count(), "{what}");
            let (v, a) = props(b);
            assert!(
                rel(v, v0 * k * k * k) < 3e-3 && rel(a, a0 * k * k) < 3e-3,
                "{what}, {ops}: volume {v} against {}, area {a} against {}",
                v0 * k * k * k,
                a0 * k * k
            );
        }
    }
}

#[test]
fn transforms_that_cannot_be_applied_leave_the_body_and_warn() {
    for (what, ops, said) in [
        ("scale of two", "    double3 xformOp:scale = (2, 2)\n    uniform token[] xformOpOrder = [\"xformOp:scale\"]".to_string(), "numbers"),
        ("NaN scale", "    double3 xformOp:scale = (nan, 1, 1)\n    uniform token[] xformOpOrder = [\"xformOp:scale\"]".to_string(), "finite"),
        (
            "projective matrix",
            "    matrix4d xformOp:transform = ((1, 0, 0, 1), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1))\n    uniform token[] xformOpOrder = [\"xformOp:transform\"]".to_string(),
            "affine",
        ),
        ("unknown operation", "    double3 xformOp:wobble = (1, 1, 1)\n    uniform token[] xformOpOrder = [\"xformOp:wobble\"]".to_string(), "not supported"),
        ("singular inverse", "    double3 xformOp:scale = (0, 1, 1)\n    uniform token[] xformOpOrder = [\"!invert!xformOp:scale\"]".to_string(), "inverted"),
        (
            "time samples",
            "    double3 xformOp:translate.timeSamples = { 0: (1, 0, 0), 1: (9, 0, 0) }\n    uniform token[] xformOpOrder = [\"xformOp:translate\"]".to_string(),
            "time samples",
        ),
    ] {
        let (step, w) = usd_to_step(&world_with(PLATE, &ops)).unwrap();
        assert!(w.iter().any(|m| m.contains(said)), "{what}: {w:?}");
        let imp = step_import(&step).unwrap();
        let (lo, hi) = bbox(&imp.bodies[0]);
        assert!(near(lo, [0.0; 3]) && near(hi, [40.0, 30.0, 10.0]), "{what}: {lo:?} {hi:?}");
    }
}

#[test]
fn transforms_that_change_shapes_are_not_read_yet() {
    let order = "    uniform token[] xformOpOrder = [\"xformOp:scale\"]";
    for scale in ["(1, 2, 3)", "(0, 0, 0)", "(1e300, 1e300, 1e300)"] {
        let text = world_with(PLATE, &format!("    double3 xformOp:scale = {scale}\n{order}"));
        let e = rejected(scale, &text);
        assert!(e.contains("not supported yet"), "{scale}: {e}");
    }
    // The other prims of the file are still read.
    let start = PLATE.find("    def BrepArray \"Brep0\"").unwrap();
    let end = PLATE.rfind("}\n").unwrap();
    let squashed = PLATE[start..end].replace("\"Brep0\"", "\"Squashed\"").replacen(
        "    {\n",
        "    {\n        double3 xformOp:scale = (1, 2, 3)\n        uniform token[] xformOpOrder = [\"xformOp:scale\"]\n",
        1,
    );
    let two = format!("{}{squashed}}}\n", &PLATE[..end]);
    let (step, w) = usd_to_step(&two).unwrap();
    assert!(w.iter().any(|m| m.contains("/World/Squashed") && m.contains("not supported yet")), "{w:?}");
    assert_eq!(step_import(&step).unwrap().bodies.len(), 1);
}

// ---- Memory ---------------------------------------------------------------------------------

/// What a small file must not be able to cost: a prim's path and its ancestors' transforms are
/// shared by the prims below them, not copied into each, and every name and transform is capped.
#[test]
fn small_files_cannot_make_large_allocations() {
    let started = std::time::Instant::now();
    // A name of 1 MiB with 10,000 empty prims under it (10 GB if each prim copied its path).
    let long = format!("#usda 1.0\ndef Xform \"{}\"\n{{\n{}}}\n", "a".repeat(1 << 20), "    def BrepArray \"b\" { }\n".repeat(10_000));
    assert!(rejected("long ancestor name", &long).contains("longer than 256 bytes"));
    let long_child = format!("#usda 1.0\ndef Xform \"x\"\n{{\n    def BrepArray \"{}\" {{ }}\n}}\n", "a".repeat(300));
    assert!(rejected("long prim name", &long_child).contains("longer than 256 bytes"));
    let long_prop = format!("#usda 1.0\ndef Xform \"x\"\n{{\n    double xformOp:{} = 1\n}}\n", "a".repeat(300));
    assert!(rejected("long property name", &long_prop).contains("longer than 256 bytes"));

    // A transform of millions of numbers, in a tuple and in a list, on an ancestor of 10,000 prims.
    let children = "    def BrepArray \"b\" { }\n".repeat(10_000);
    let numbers = vec!["0"; 3_000_000].join(",");
    for value in [format!("({numbers})"), format!("[{numbers}]")] {
        let text = format!(
            "#usda 1.0\ndef Xform \"x\"\n{{\n    matrix4d xformOp:transform = {value}\n    uniform token[] xformOpOrder = [\"xformOp:transform\"]\n{children}}}\n"
        );
        assert!(rejected("a huge matrix", &text).contains("too many numbers"));
    }
    // The same in a matrix of 17 numbers, a list of 65 names, a name of 257 bytes, and 65 operations.
    let m17 = vec!["0"; 17].join(",");
    let text = format!("#usda 1.0\ndef Xform \"x\"\n{{\n    double3 xformOp:scale = ({m17})\n}}\n");
    assert!(rejected("17 numbers", &text).contains("too many numbers"));
    let names65 = vec!["\"xformOp:translate\""; 65].join(",");
    let text = format!("#usda 1.0\ndef Xform \"x\"\n{{\n    uniform token[] xformOpOrder = [{names65}]\n}}\n");
    assert!(rejected("65 names", &text).contains("too many strings"));
    let text = format!("#usda 1.0\ndef Xform \"x\"\n{{\n    uniform token[] xformOpOrder = [\"{}\"]\n}}\n", "a".repeat(257));
    assert!(rejected("a long operation name", &text).contains("too long"));
    let ops: String = (0..65).map(|i| format!("    double xformOp:rotateZ:n{i} = 1\n")).collect();
    let text = format!("#usda 1.0\ndef Xform \"x\"\n{{\n{ops}}}\n");
    assert!(rejected("65 operations", &text).contains("transform operations"));
    // A name of 300 bytes in `customData` is cut, not copied into every body's name.
    let named = edit(
        CUBE,
        "customData = {\n            string source = \"\"\n        }",
        &format!(
            "customData = {{\n            dictionary solidcraft = {{\n                string name = \"{}\"\n            }}\n        }}",
            "n".repeat(300)
        ),
    );
    assert_eq!(import(&named).bodies[0].name.len(), 256);

    // The most a file can nest: 60 ancestors with names of 256 bytes and a transform each, and
    // 10,000 prims at the bottom. Each ancestor is held once, and each prim's transform comes
    // from its parent's.
    let name = "a".repeat(256);
    let mut text = String::from("#usda 1.0\n");
    for _ in 0..60 {
        text.push_str(&format!(
            "def Xform \"{name}\"\n{{\n    double3 xformOp:translate = (1, 0, 0)\n    uniform token[] xformOpOrder = [\"xformOp:translate\"]\n"
        ));
    }
    text.push_str(&children);
    text.push_str(&"}\n".repeat(60));
    let layer = usda::parse(&text).unwrap();
    assert_eq!(layer.breps.len(), 10_000);
    assert_eq!(layer.node_count(), 60 + 10_000);
    assert!(layer.path(layer.breps[0].node).len() > 60 * 256);
    // Reading them all takes one product per prim, not one per ancestor and prim.
    assert!(rejected("empty prims", &text).contains("nothing to read"));
    assert!(started.elapsed().as_secs() < 20, "took {:?}", started.elapsed());
}
