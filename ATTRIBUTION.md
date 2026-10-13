# Attribution

Every non-code asset in this repository (images, icons, fonts, example parts) is listed here with
its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset
file is missing from this table.

**Policy (mandatory):** SolidCraft contains **no Autodesk iconography, images, artwork, fonts,
templates, materials or help text.** Every asset is original work by SolidCraft contributors or
third-party material under an open licence. Screenshots of Autodesk software are never committed.

Generated-in-code assets are original and have no file to list:
- the UI icon set (`crates/ui-egui/src/icons.rs`);
- the axis view, navigation bar and timeline glyphs (`crates/ui-egui/src/viewport.rs`,
  `crates/ui-egui/src/timeline.rs`).

## Files

| File | Author | Source | Licence |
|---|---|---|---|
| assets/icon.svg | SolidCraft contributors | original macOS adaptation of the code-drawn SolidCraft mark, edited and exported with VectorCraft; Apple HIG and production templates consulted as layout references only | MIT OR Apache-2.0 |
| assets/SolidCraft.icon/icon.json | SolidCraft contributors | approved Icon Composer composition and appearance settings for the original SolidCraft artwork | MIT OR Apache-2.0 |
| assets/SolidCraft.icon/Assets/02-grid.svg | SolidCraft contributors | original perspective grid with elliptical radial alpha, exported from VectorCraft | MIT OR Apache-2.0 |
| assets/SolidCraft.icon/Assets/02-grid-dark.svg | SolidCraft contributors | original blue perspective grid with elliptical radial alpha, exported from VectorCraft for the Dark appearance | MIT OR Apache-2.0 |
| assets/SolidCraft.icon/Assets/03-solid-blue-cutout.svg | SolidCraft contributors | original saturated blue CAD solid with transparent topology cutouts, edited and exported with VectorCraft | MIT OR Apache-2.0 |
| assets/Assets.car | SolidCraft contributors | Apple actool compilation of assets/SolidCraft.icon for macOS; contains original artwork only | MIT OR Apache-2.0 |
| assets/icon.vectorcraft | SolidCraft contributors | editable VectorCraft source for assets/icon.svg, with named face and background layers | MIT OR Apache-2.0 |
| assets/icon.png | SolidCraft contributors | 1024 × 1024 transparent sRGB export of assets/icon.svg from VectorCraft | MIT OR Apache-2.0 |
| assets/icon.icns | SolidCraft contributors | macOS fallback compiled by Apple's actool from the approved assets/SolidCraft.icon composition | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_16x16.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_16x16@2x.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_32x32.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_32x32@2x.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_128x128.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_128x128@2x.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_256x256.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_256x256@2x.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_512x512.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| assets/icon.iconset/icon_512x512@2x.png | SolidCraft contributors | native sips export from assets/icon.png | MIT OR Apache-2.0 |
| docs/screenshots/sample-plate.png | SolidCraft contributors | screenshot of SolidCraft itself (`solidcraft --sample`) | MIT OR Apache-2.0 |
| docs/screenshots/enclosure-demo-0.2.1.gif | SolidCraft contributors | original electronics enclosure screen recording of the actual SolidCraft 0.2.1 English desktop UI, generated with examples/demo/record_ui.sh and its original animated pointer overlay (4× playback) | MIT OR Apache-2.0 |
| docs/screenshots/enclosure-demo-0.2.1.mp4 | SolidCraft contributors | original electronics enclosure screen recording of the actual SolidCraft 0.2.1 English desktop UI, generated with examples/demo/record_ui.sh and its original animated pointer overlay (4× playback) | MIT OR Apache-2.0 |
| docs/book/src/og.png | SolidCraft contributors | original 1280 × 640 social preview card (the SolidCraft mark, name and tagline), served by the docs site for link previews | MIT OR Apache-2.0 |
| docs/book/src/generated/sample-plate.png | SolidCraft contributors | copy of docs/screenshots/sample-plate.png made by `cargo xtask book` (gitignored) | MIT OR Apache-2.0 |
| docs/book/src/generated/bracket.png | SolidCraft contributors | rendered by `cargo xtask book` (`solidcraft-cli snapshot examples/bracket.json`; gitignored) | MIT OR Apache-2.0 |
| docs/book/src/generated/enclosure.png | SolidCraft contributors | rendered by `cargo xtask book` (`solidcraft-cli snapshot docs/book/scripts/enclosure.json`; gitignored) | MIT OR Apache-2.0 |
| docs/book/src/generated/revolve.png | SolidCraft contributors | rendered by `cargo xtask book` (`solidcraft-cli snapshot docs/book/scripts/revolve.json`; gitignored) | MIT OR Apache-2.0 |
| docs/book/src/generated/sheet-metal.png | SolidCraft contributors | rendered by `cargo xtask book` (`solidcraft-cli snapshot docs/book/scripts/sheet-metal.json`; gitignored) | MIT OR Apache-2.0 |
| crates/sketch/fonts/LiberationSans-Regular.ttf | Red Hat, Inc. (Liberation Fonts 2.x; digitized data Google) | https://github.com/liberationfonts (Debian fonts-liberation), unmodified; licence text in crates/sketch/fonts/OFL.txt; used for sketch Text outlines | SIL OFL 1.1 |
| apps/solidcraft-web/fonts/BIZUDPGothic-Regular.ttf | The BIZ UDGothic Project Authors | https://github.com/googlefonts/morisawa-biz-ud-gothic (Google Fonts), unmodified; licence text in apps/solidcraft-web/fonts/OFL.txt; used for web UI Japanese font | SIL OFL 1.1 |
| tests/oracle/17-loft/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/17-loft/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/23-bracket-multifeature/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/23-bracket-multifeature/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/28-blind-pocket-fillets/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/28-blind-pocket-fillets/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/30-fillet-after-shell/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/30-fillet-after-shell/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/31-fillet-l-all-edges/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/31-fillet-l-all-edges/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/34-chamfer-then-fillet/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/34-chamfer-then-fillet/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/35-fillet-curved-curved/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/35-fillet-curved-curved/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/36-corner-blend-two-radii/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/36-corner-blend-two-radii/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/37-pyramid-apex-blend/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/37-pyramid-apex-blend/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/39-tee-pipe/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/39-tee-pipe/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/41-tube-radial-holes/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/41-tube-radial-holes/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/42-overlapping-patterns/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/42-overlapping-patterns/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/47-draft-curved-pocket/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/47-draft-curved-pocket/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/51-loft-rect-circle-offset/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/51-loft-rect-circle-offset/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/56-holes-on-curved-face/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/56-holes-on-curved-face/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/58-phone-stand/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/58-phone-stand/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/59-enclosure-base/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/59-enclosure-base/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/61-shaft-keyway/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/61-shaft-keyway/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/63-hinge-knuckle/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/63-hinge-knuckle/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| tests/oracle/65-sketch-gear-tooth/reference.stl | SolidCraft contributors | triangulation of our own recipe (tests/oracle/65-sketch-gear-tooth/recipe.json) built with FreeCAD as an external tool by tools/freecad-oracle | MIT OR Apache-2.0 |
| crates/kernel/src/usd/testdata/Cube.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/Cube.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/CubeBrepArray.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/CubeBrepArray.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/cubeWithVoid.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/cubeWithVoid.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/Cube_Roundtrip.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/Cube_Roundtrip.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/CubeFaceMaterials.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/CubeFaceMaterials.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/nonManifoldCubes.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/nonManifoldCubes.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/Cone_SmConnect_Brep.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/Cone_SmConnect_Brep.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/mouse.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/brep_validator/mouse.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/sphere_nurbs.usda | NVIDIA CORPORATION & AFFILIATES | `TestFiles/usd_TestFiles/sphere_nurbs.usda` of https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979), copied unmodified; test input for the USD BrepArray importer | Apache-2.0 |
| crates/kernel/src/usd/testdata/analytic_cube.usda | NVIDIA CORPORATION & AFFILIATES and SolidCraft contributors | `Cube.usda` above with its NURBS surfaces and edge curves replaced by planes and lines (same topology); derived from https://github.com/NVIDIA-Omniverse/usd-brep (release 1.0.0, commit c9c5979) | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_box.usda | NVIDIA CORPORATION & AFFILIATES | unit box written by the usd-brep library with `create_box` and `export_brep` (library from https://github.com/NVIDIA-Omniverse/usd-brep) | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_plate_hole.usda | NVIDIA CORPORATION & AFFILIATES (library output) | box 40 x 30 x 10 minus a cylinder of radius 5, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_cylinder.usda | NVIDIA CORPORATION & AFFILIATES (library output) | cylinder of radius 5 and height 12, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_cone.usda | NVIDIA CORPORATION & AFFILIATES (library output) | cone of base radius 4 and height 9, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_frustum.usda | NVIDIA CORPORATION & AFFILIATES (library output) | cone frustum of radii 4 and 2, height 9, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_box_fillet.usda | NVIDIA CORPORATION & AFFILIATES (library output) | box 20 with all edges rounded by 3, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_box_cone_cut.usda | NVIDIA CORPORATION & AFFILIATES (library output) | box 30 x 30 x 20 minus a cone, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_box_sphere_cut.usda | NVIDIA CORPORATION & AFFILIATES (library output) | box 30 x 30 x 20 minus a sphere of radius 8, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_box_cylinder_union.usda | NVIDIA CORPORATION & AFFILIATES (library output) | box 10 joined with a cylinder of radius 3, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_sphere.usda | NVIDIA CORPORATION & AFFILIATES (library output) | sphere of radius 10, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_torus.usda | NVIDIA CORPORATION & AFFILIATES (library output) | torus of radii 20 and 4, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_partial_cylinder.usda | NVIDIA CORPORATION & AFFILIATES (library output) | partial cylinder, 30 to 250 degrees, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_partial_cone.usda | NVIDIA CORPORATION & AFFILIATES (library output) | partial cone frustum, 0 to 200 degrees, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_partial_sphere.usda | NVIDIA CORPORATION & AFFILIATES (library output) | partial sphere, 20 to 300 degrees, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_partial_torus.usda | NVIDIA CORPORATION & AFFILIATES (library output) | partial torus, 0 to 210 degrees, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_pyramid.usda | NVIDIA CORPORATION & AFFILIATES (library output) | pyramid, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_hollow_cylinder.usda | NVIDIA CORPORATION & AFFILIATES (library output) | cylinder of radius 10 minus a cylinder of radius 6, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_box_through_sphere.usda | NVIDIA CORPORATION & AFFILIATES (library output) | box 30 x 30 x 20 minus a sphere of radius 12, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_cylinder_fillet.usda | NVIDIA CORPORATION & AFFILIATES (library output) | cylinder of radius 8 with its end edges rounded by 2, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |
| crates/kernel/src/usd/testdata/lib_two_bodies.usda | NVIDIA CORPORATION & AFFILIATES (library output) | a box, a cylinder and a sphere in one prim, built and written by the usd-brep Python library 1.0.0 (`pip install usd-brep`, https://github.com/NVIDIA-Omniverse/usd-brep); generated for this test | Apache-2.0 |

The `.usda` files in `crates/kernel/src/usd/testdata/` are test inputs from NVIDIA's usd-brep project, kept under its Apache-2.0 licence (text in LICENSE-APACHE) with NVIDIA's copyright. The fixtures are taken from that repository as they are (analytic_cube.usda is derived from one of them); the lib_* files are output of its library; no SolidCraft contributor made them with a commercial CAD product.

## Code adapted from sibling projects

| SolidCraft file | Adapted from | Licence |
|---|---|---|
| xtask/src/layers.rs | CADCraft `xtask/src/layers.rs` (dependency layering rules and tests) | MIT OR Apache-2.0 |
| apps/solidcraft/src/control_server.rs | CADCraft `apps/cadcraft/src/control_server.rs` (loopback JSON-lines server) | MIT OR Apache-2.0 |
| crates/ui-egui/src/lib.rs (screenshot queue, synthetic input) | CADCraft `crates/ui-egui/src/lib.rs` | MIT OR Apache-2.0 |
| crates/ui-egui/src/control.rs (method dispatch and screenshot saving) | CADCraft `crates/ui-egui/src/control.rs` | MIT OR Apache-2.0 |
| crates/ui-egui/src/gpu.rs (paint-callback structure) | CADCraft `crates/ui-egui/src/gpu.rs` | MIT OR Apache-2.0 |
| crates/mcp/src/server.rs, crates/mcp/src/backend.rs (JSON-RPC framing, MCP lifecycle, control-port bridge) | CADCraft `crates/mcp/src/{server,backend}.rs` | MIT OR Apache-2.0 |
| crates/mcp/src/tools.rs (tool schema helpers and argument checking) | CADCraft `crates/mcp/src/tools.rs`, GridCraft `crates/mcp/src/tools.rs` | MIT OR Apache-2.0 |
| crates/io/src/threemf.rs (bounded zip part reading and writing) | GridCraft `crates/xlsx/src/package.rs` | MIT OR Apache-2.0 |
| crates/ui-egui/src/titlebar.rs (title bar drag, caption buttons, resize zones, their tests) and the window setup in apps/solidcraft/src/main.rs | VectorCraft `crates/ui-egui/src/titlebar.rs`, `apps/vectorcraft/src/main.rs` | MIT OR Apache-2.0 |
| LICENSE-MIT, LICENSE-APACHE (licence text) | CADCraft | — |
| .github/workflows/release.yml, .github/workflows/packaging-lint.yml and packaging/ (release pipeline: Linux AppImage/deb/rpm/Flatpak/tar.gz, FreeBSD, macOS dmg, Windows MSI/portable zip, web zip; AppRun, nfpm, Flatpak, WiX and Info.plist templates) | PhotoCraft (https://github.com/storytold/photocraft) `.github/workflows/{release,packaging-lint}.yml`, `packaging/` | Apache-2.0 |

## Recovered contributions

| SolidCraft file | Original contribution | Licence |
|---|---|---|
| crates/ui-egui/src/viewport.rs (`nearest_quarter_turn`, `axis_view_orbit`) | N3MI-DG's [PR #101](https://github.com/bherbruck/solidcraft/pull/101): [minimal TOP/BOTTOM turns](https://github.com/bherbruck/solidcraft/commit/c28e780f52490784ef5886f094b9d3f6d10c3e3f) and [drag-to-orbit](https://github.com/bherbruck/solidcraft/commit/f30b30be6ecc4e2e6544a851243137c01a354934), adapted to the axis gizmo | MIT OR Apache-2.0 |

## Vendored third-party code

| Directory | Source | Licence | Changes |
|---|---|---|---|
| vendor/truck-meshalgo | truck-meshalgo 0.4.0, https://github.com/ricosjp/truck (RICOS Co. Ltd.) | Apache-2.0 | meshing: parameter searches kept inside the surface domain (vendor/README.md) |
| vendor/truck-stepio | truck-stepio 0.3.0, https://github.com/ricosjp/truck (RICOS Co. Ltd.) | Apache-2.0 | STEP writer: intersection-curve surfaces written at unique ids (vendor/README.md) |
| vendor/truck-shapeops | truck-shapeops 0.4.0, https://github.com/ricosjp/truck (RICOS Co. Ltd.) | Apache-2.0 | robust meshing and nearest-parameter fallback in booleans; degenerate intersection-curve rejection; face and edge history of booleans; trace macro (vendor/README.md) |
| vendor/matext4cgmath | matext4cgmath 0.1.0, https://github.com/ricosjp/matext4cgmath (RICOS Co. Ltd.) | Apache-2.0 | root refinement stops after 100 Newton steps (vendor/README.md) |

## Vendored renderer compatibility fix

`vendor/egui-wgpu` is egui-wgpu 0.36.2 from https://github.com/emilk/egui
(MIT OR Apache-2.0; both original licence texts are included). Rust sources and
its original `egui.wgsl` shader are retained; no images or other artwork are copied.
SolidCraft's narrow patch selects a safe 1x MSAA path on the chosen OpenGL adapter
and shares that effective count with the surface attachments and CAD callbacks.
Vulkan, DirectX 12, Metal and WebGPU retain their requested multisampling.
