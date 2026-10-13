//! Documents and files: new, open (designs and STEP), insert STEP, save, export.

use serde_json::{Value, json};
use solidcraft_doc::Document;
use solidcraft_io::Format;

use super::CommandSpec;
use crate::params::{bad, bool_, str_, string_list};
use crate::{EngineError, Result, Session};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("file.new", "New Design", new_doc).icon("new").key("Ctrl+N").noundo().params("name?"),
    CommandSpec::new("doc.open", "Open", open)
        .icon("open")
        .key("Ctrl+O")
        .noundo()
        .params("path: .solidcraft design, or a .step/.stp, .igs/.iges or usd-brep .usda/.usd file (opens as a new design)"),
    CommandSpec::new("file.insert_step", "Insert STEP", insert_step)
        .at("SOLID", "INSERT")
        .icon("import")
        .params("path: .step/.stp, .igs/.iges or usd-brep .usda/.usd file (its bodies join the design as an Import base feature); name?"),
    CommandSpec::new("file.insert_mesh", "Insert Mesh", insert_mesh)
        .at("SOLID", "INSERT")
        .icon("import")
        .params("path: .3mf or .stl file (its meshes join the design as mesh bodies); name?"),
    CommandSpec::new("file.save", "Save", save)
        .icon("save")
        .key("Ctrl+S")
        .noundo()
        .params("path? (default: current file); format?: container|json (default: json for .json, otherwise container); compression?: fast|compact (default: fast); preview?: bool (experimental saved picture, default: false)"),
    CommandSpec::new("file.save_as", "Save As", save_as)
        .icon("save")
        .noundo()
        .params("path; format?: container|json (default: json for .json, otherwise container); compression?: fast|compact (default: fast); preview?: bool (experimental saved picture, default: false)"),
    CommandSpec::new("doc.recovery_list", "Recoverable Designs", recovery_list)
        .noundo()
        .params("dir? (default: the recovery folder) → designs autosaved by apps that crashed or closed with unsaved changes"),
    CommandSpec::new("doc.recover", "Recover Design", recover)
        .noundo()
        .params("id (from doc.recovery_list); dir?; discard?: bool (delete the entry once open) — opens it with its file path, unsaved"),
    CommandSpec::new("doc.recovery_discard", "Discard Recovered Design", recovery_discard).noundo().params("id, or all: true; dir?"),
    CommandSpec::new("file.export", "Export", export)
        .icon("export")
        .noundo()
        .params("path; format?: stl|stla|obj|step|iges|usda|3mf|json (default from extension); bodies?: [names] (JSON exports the complete design)"),
    CommandSpec::new("file.save_mesh", "Save As Mesh", save_stl).icon("export").noundo().params("path; bodies?: [names]; ascii?: bool"),
];

fn new_doc(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_(p, "name").unwrap_or("Untitled");
    *s = Session::new(Document::new(name));
    Ok(json!({"name": name}))
}

fn path_arg<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    str_(p, "path").filter(|x| !x.trim().is_empty() && x.len() < 4096).ok_or_else(|| bad(cmd, "`path` is required"))
}

/// Read a STEP file as an Import feature.
fn read_step(path: &str) -> Result<solidcraft_io::StepFeature> {
    let bytes = solidcraft_io::vfs::read(path, solidcraft_io::MAX_STEP_BYTES).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    solidcraft_io::step_import_feature(&bytes, path).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

/// Add an Import feature for a STEP file to the session's design.
fn add_step(s: &mut Session, path: &str, name: Option<&str>) -> Result<Value> {
    let sf = read_step(path)?;
    let fname = name.filter(|n| !n.trim().is_empty()).unwrap_or(&sf.name).to_string();
    let id = s.doc_mut().add_feature(sf.kind, Some(&fname))?;
    if let Some(f) = s.doc_mut().feature_mut(id) {
        f.body_names = sf.body_names;
    }
    s.active_sketch = None;
    s.refresh();
    if let Some(e) = s.model.result(id).and_then(|r| r.error.clone()) {
        return Err(EngineError::Other(e));
    }
    let st = s.model.state();
    let bodies: Vec<&str> = st.bodies.iter().filter(|b| b.feature == id).map(|b| b.name.as_str()).collect();
    let fname = s.doc.feature(id).map(|f| f.name.clone()).unwrap_or_default();
    Ok(json!({"path": path, "feature": id, "name": fname, "bodies": bodies, "warnings": sf.warnings}))
}

/// Add a MeshImport feature for a 3MF or STL file.
fn add_mesh(s: &mut Session, path: &str, name: Option<&str>) -> Result<Value> {
    let bytes = solidcraft_io::vfs::read(path, solidcraft_io::MAX_3MF_BYTES).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let mf = solidcraft_io::mesh_import_feature(&bytes, path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let fname = name.filter(|n| !n.trim().is_empty()).unwrap_or(&mf.name).to_string();
    let id = s.doc_mut().add_feature(mf.kind, Some(&fname))?;
    if let Some(f) = s.doc_mut().feature_mut(id) {
        f.body_names = mf.body_names;
    }
    s.active_sketch = None;
    s.refresh();
    if let Some(e) = s.model.result(id).and_then(|r| r.error.clone()) {
        return Err(EngineError::Other(e));
    }
    let st = s.model.state();
    let bodies: Vec<&str> = st.bodies.iter().filter(|b| b.feature == id).map(|b| b.name.as_str()).collect();
    let fname = s.doc.feature(id).map(|f| f.name.clone()).unwrap_or_default();
    Ok(json!({"path": path, "feature": id, "name": fname, "bodies": bodies, "warnings": mf.warnings}))
}

fn insert_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.insert_mesh";
    let path = path_arg(p, cmd)?;
    if !solidcraft_io::is_mesh_path(path) {
        return Err(bad(cmd, "only 3MF and STL files (.3mf, .stl) can be inserted as meshes"));
    }
    add_mesh(s, path, str_(p, "name"))
}

fn insert_step(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.insert_step";
    let path = path_arg(p, cmd)?;
    if !solidcraft_io::is_step_path(path) {
        return Err(bad(cmd, "only STEP, IGES and usd-brep files (.step, .stp, .igs, .iges, .usda, .usd) can be inserted"));
    }
    add_step(s, path, str_(p, "name"))
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_arg(p, "doc.open")?;
    if solidcraft_io::is_step_path(path) || solidcraft_io::is_mesh_path(path) {
        // A new, unsaved design holding the file's bodies.
        let stem = std::path::Path::new(path).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        let mut fresh = Session::new(Document::new(&stem));
        let r = if solidcraft_io::is_step_path(path) { add_step(&mut fresh, path, None)? } else { add_mesh(&mut fresh, path, None)? };
        fresh.undo.clear();
        *s = fresh;
        let errors = s.model.results.iter().filter(|r| r.error.is_some()).count();
        return Ok(json!({"path": path, "features": s.doc.features.len(), "errors": errors, "bodies": r["bodies"], "warnings": r["warnings"]}));
    }
    let bytes = solidcraft_io::vfs::read(path, solidcraft_io::MAX_DESIGN_BYTES).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let doc = solidcraft_io::read_design(&bytes)?;
    if let Some(notify) = s.open_preview.take()
        && let Some(preview) = solidcraft_io::read_design_preview(&bytes)
    {
        notify(preview);
    }
    *s = Session::new(doc);
    s.path = Some(path.to_string());
    let errors = s.model.results.iter().filter(|r| r.error.is_some()).count();
    Ok(json!({"path": path, "features": s.doc.features.len(), "errors": errors}))
}

fn write(s: &mut Session, path: &str, format: &str, compression: &str, preview: bool) -> Result<Value> {
    let compression = match compression {
        "fast" => solidcraft_io::DesignCompression::Fast,
        "compact" => solidcraft_io::DesignCompression::Compact,
        _ => return Err(bad("file.save", "compression must be fast or compact")),
    };
    let bytes = match format {
        "container" => solidcraft_io::write_design_container_with_preview(&s.doc, compression, preview.then(|| saved_preview(s)).flatten().as_ref())?,
        "json" => solidcraft_io::write_design(&s.doc),
        _ => return Err(bad("file.save", "format must be container or json")),
    };
    // Atomic, keeping the version being replaced as `<file>.bak`.
    solidcraft_io::write_atomic(std::path::Path::new(path), &bytes, true).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    s.path = Some(path.to_string());
    s.mark_saved();
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

/// Cache only warm display geometry. A save never waits for an unevaluated mesh.
fn saved_preview(s: &Session) -> Option<solidcraft_io::DesignPreview> {
    if s.active_sketch.is_some() || !s.pending_moves.is_empty() || s.model.results.iter().any(|r| r.error.is_some()) {
        return None;
    }
    let world = s.cached_world_state()?;
    if world.bodies.is_empty() || world.bodies.len() > 1024 {
        return None;
    }
    let mut preview = solidcraft_io::DesignPreview::default();
    let mut bytes = 0usize;
    let mut triangles = 0usize;
    for body in &world.bodies {
        let mesh = body.cached_mesh()?;
        bytes =
            bytes.saturating_add(mesh.positions.len().saturating_mul(24)).saturating_add(mesh.triangles.len().saturating_mul(12)).saturating_add(24);
        triangles = triangles.saturating_add(mesh.triangles.len());
        if bytes > 8 << 20 || triangles > 100_000 {
            return None;
        }
        preview.meshes.push(solidcraft_doc::MeshData {
            name: body.name.clone(),
            positions: mesh.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect(),
            triangles: mesh.triangles.iter().flatten().copied().collect(),
            color: s.doc.body_color(body),
        });
    }
    Some(preview)
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = match str_(p, "path") {
        Some(x) if !x.trim().is_empty() => x.to_string(),
        _ => s.path.clone().ok_or_else(|| bad("file.save", "the design has no file yet: give a `path`"))?,
    };
    write(
        s,
        &path,
        str_(p, "format").unwrap_or(default_design_format(&path)),
        str_(p, "compression").unwrap_or("fast"),
        bool_(p, "preview").unwrap_or(false),
    )
}

fn default_design_format(path: &str) -> &'static str {
    if std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("json")) { "json" } else { "container" }
}

fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_arg(p, "file.save_as")?.to_string();
    write(
        s,
        &path,
        str_(p, "format").unwrap_or(default_design_format(&path)),
        str_(p, "compression").unwrap_or("fast"),
        bool_(p, "preview").unwrap_or(false),
    )
}

fn export_to(s: &Session, path: &str, format: Format, bodies: &[String]) -> Result<Value> {
    let name = std::path::Path::new(path).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| s.doc.name.clone());
    // A design with components exports as a STEP assembly; otherwise bodies as placed.
    let bytes = if format == Format::Design {
        if !bodies.is_empty() {
            return Err(bad("file.export", "design JSON exports the complete document; omit bodies"));
        }
        solidcraft_io::write_design(&s.doc)
    } else if format == Format::Step && bodies.is_empty() && !s.doc.occurrences.is_empty() {
        solidcraft_io::step_assembly(&s.doc, &s.doc.painted(&s.model.state()), &name)?
    } else {
        // Bodies carry the colour they are shown in (appearance, material or imported).
        solidcraft_io::export(&s.doc.painted(&s.world_state()), bodies, format, &name)?
    };
    solidcraft_io::vfs::write(path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "bytes": bytes.len(), "format": format!("{format:?}")}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_arg(p, "file.export")?;
    let format = Format::from_name(str_(p, "format").unwrap_or(path))?;
    export_to(s, path, format, &string_list(p, "bodies"))
}

fn save_stl(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_arg(p, "file.save_mesh")?;
    let f = if bool_(p, "ascii").unwrap_or(false) { Format::StlAscii } else { Format::StlBinary };
    export_to(s, path, f, &string_list(p, "bodies"))
}

fn recovery_dir(p: &Value, cmd: &str) -> Result<std::path::PathBuf> {
    match str_(p, "dir").filter(|d| !d.trim().is_empty() && d.len() < 4096) {
        Some(d) => Ok(d.into()),
        None => crate::recovery::default_dir().ok_or_else(|| bad(cmd, "no recovery folder on this system (give `dir`)")),
    }
}

fn recovery_list(_s: &mut Session, p: &Value) -> Result<Value> {
    let dir = recovery_dir(p, "doc.recovery_list")?;
    Ok(json!({"dir": dir, "designs": crate::recovery::orphans(&dir)}))
}

fn recover(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "doc.recover";
    let dir = recovery_dir(p, cmd)?;
    let id = str_(p, "id").ok_or_else(|| bad(cmd, "`id` is required (see doc.recovery_list)"))?;
    let (doc, entry) = crate::recovery::load(&dir, id)?;
    *s = Session::new(doc);
    s.path = entry.path.clone();
    s.mark_unsaved();
    if bool_(p, "discard").unwrap_or(false) {
        crate::recovery::remove(&dir, id);
    }
    let errors = s.model.results.iter().filter(|r| r.error.is_some()).count();
    Ok(json!({"id": id, "path": entry.path, "name": entry.name, "features": s.doc.features.len(), "errors": errors}))
}

fn recovery_discard(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "doc.recovery_discard";
    let dir = recovery_dir(p, cmd)?;
    let ids: Vec<String> = if bool_(p, "all").unwrap_or(false) {
        crate::recovery::orphans(&dir).into_iter().map(|e| e.id).collect()
    } else {
        vec![str_(p, "id").ok_or_else(|| bad(cmd, "`id` or `all` is required"))?.to_string()]
    };
    for id in &ids {
        crate::recovery::remove(&dir, id);
    }
    Ok(json!({"discarded": ids}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual warmed engine save and provisional-open benchmark"]
    fn benchmark_warm_design_save_with_preview() {
        let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("target"));
        // The mesh storage fixture intentionally does not evaluate geometry. Use
        // original valid STEP boxes for the engine/display-cache measurement.
        let input = target.join("issue303-step.json");
        let doc = solidcraft_io::read_design(&std::fs::read(input).unwrap()).unwrap();
        let mut session = Session::new(doc);
        for body in &session.world_state().bodies {
            body.mesh();
        }
        let mut samples = [Vec::new(), Vec::new()];
        for round in 0..9 {
            for index in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
                let path = target.join(if index == 0 { "issue303-warm.json" } else { "issue303-warm.solidcraft" });
                let began = std::time::Instant::now();
                session.execute("file.save", &json!({"path":path,"preview":true})).unwrap();
                samples[index].push(began.elapsed().as_secs_f64() * 1000.0);
            }
        }
        for s in &mut samples {
            s.sort_by(f64::total_cmp);
        }
        let bytes = std::fs::read(target.join("issue303-warm.solidcraft")).unwrap();
        assert!(solidcraft_io::read_design_preview(&bytes).is_some());
        assert_eq!(solidcraft_io::read_design(&bytes).unwrap(), *session.doc);
        let mut reads = Vec::new();
        for _ in 0..9 {
            let began = std::time::Instant::now();
            assert!(solidcraft_io::read_design_preview(&bytes).is_some());
            reads.push(began.elapsed().as_secs_f64() * 1000.0);
        }
        reads.sort_by(f64::total_cmp);
        eprintln!(
            "warmed engine median atomic save ms json={:.3} container_with_preview={:.3}; preview_decode_ms={:.3}; container_bytes={}",
            samples[0][4],
            samples[1][4],
            reads[4],
            bytes.len()
        );
    }

    #[test]
    fn save_uses_only_warm_meshes_and_open_preview_does_not_replace_evaluation() {
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":10,"width":20,"height":30})).unwrap();
        assert!(saved_preview(&s).is_none(), "save must not triangulate a cold body");
        let state = s.world_state();
        for body in &state.bodies {
            body.mesh();
        }
        let save_path = std::env::temp_dir().join(format!("solidcraft-preview-save-{}.solidcraft", std::process::id()));
        s.execute("file.save", &json!({"path":save_path})).unwrap();
        assert!(solidcraft_io::read_design_preview(&std::fs::read(&save_path).unwrap()).is_none(), "ordinary saves omit redundant display data");
        s.execute("file.save", &json!({"path":save_path,"preview":true})).unwrap();
        assert!(solidcraft_io::read_design_preview(&std::fs::read(&save_path).unwrap()).is_some());
        let _ = std::fs::remove_file(&save_path);
        let _ = std::fs::remove_file(format!("{}.bak", save_path.display()));
        let mut preview = saved_preview(&s).unwrap();
        // A checksum-valid but forged cached shape must not become editable geometry.
        for mesh in &mut preview.meshes {
            for coordinate in &mut mesh.positions {
                *coordinate *= 10.0;
            }
        }
        let bytes = solidcraft_io::write_design_container_with_preview(&s.doc, solidcraft_io::DesignCompression::Fast, Some(&preview)).unwrap();
        let path = std::env::temp_dir().join(format!("solidcraft-preview-{}.solidcraft", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let callback_seen = seen.clone();
        let mut reopened = Session {
            open_preview: Some(Box::new(move |preview| {
                callback_seen.fetch_add(preview.meshes.len(), std::sync::atomic::Ordering::Relaxed);
            })),
            ..Session::default()
        };
        reopened.execute("doc.open", &json!({"path":path})).unwrap();
        assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(reopened.open_preview.is_none());
        let measured = reopened.execute("inspect.measure", &json!({})).unwrap();
        assert!((measured["bodies"][0]["volume_mm3"].as_f64().unwrap() - 6000.0).abs() < 1e-6);
        assert_eq!(*reopened.doc, *s.doc);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn preview_saves_do_not_build_cold_placements_or_persist_transient_edits() {
        let mut s = Session::default();
        let created = s.execute("component.create", &json!({"name":"PreviewPart"})).unwrap();
        let occurrence = created["occurrence"].as_u64().unwrap();
        s.execute("solid.box", &json!({"length":2,"width":3,"height":4,"body_name":"Block","operation":"new"})).unwrap();
        for body in &s.model.state().bodies {
            body.mesh();
        }
        *s.world_cache.lock().unwrap() = None;
        assert!(saved_preview(&s).is_none());
        assert!(s.world_cache.lock().unwrap().is_none(), "saving must not prepare cold placements");
        for body in &s.world_state().bodies {
            body.mesh();
        }
        assert!(saved_preview(&s).is_some());
        s.execute("occurrence.move", &json!({"occurrence":occurrence,"translate":[10,0,0],"capture":false})).unwrap();
        for body in &s.world_state().bodies {
            body.mesh();
        }
        assert!(!s.pending_moves.is_empty());
        assert!(saved_preview(&s).is_none(), "uncaptured positions are not in the saved document");
        s.execute("component.capture_position", &json!({})).unwrap();
        for body in &s.world_state().bodies {
            body.mesh();
        }
        let preview = saved_preview(&s).unwrap();
        let min_x = preview.meshes.iter().flat_map(|mesh| mesh.positions.as_chunks::<3>().0.iter().map(|p| p[0])).fold(f64::INFINITY, f64::min);
        assert!((min_x - 10.0).abs() < 1e-9);
        let mut editing = s.scratch();
        editing.execute("sketch.create", &json!({"plane":"XY"})).unwrap();
        for body in &editing.world_state().bodies {
            body.mesh();
        }
        assert!(editing.active_sketch.is_some());
        assert!(saved_preview(&editing).is_none());
    }

    #[test]
    fn design_save_formats_reopen_and_preserve_atomic_backups() {
        let dir = std::env::temp_dir().join(format!("solidcraft-container-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let packaged = dir.join("design.solidcraft");
        let plain = dir.join("design.json");
        let export = dir.join("export.json");
        let mut session = Session::default();
        session.execute("solid.box", &json!({"length":10,"width":20,"height":30})).unwrap();
        session.execute("file.save_as", &json!({"path":packaged})).unwrap();
        let original = std::fs::read(&packaged).unwrap();
        assert!(solidcraft_io::is_design_container(&original));
        session.execute("file.save_as", &json!({"path":plain})).unwrap();
        assert!(std::fs::read_to_string(&plain).unwrap().contains("solidcraft/"));
        let path = session.path.clone();
        session.execute("file.export", &json!({"path":export,"format":"json"})).unwrap();
        assert_eq!(session.path, path);
        for file in [&packaged, &plain, &export] {
            let mut reopened = Session::default();
            reopened.execute("doc.open", &json!({"path":file})).unwrap();
            assert_eq!(reopened.doc, session.doc);
            let measure = reopened.execute("inspect.measure", &json!({})).unwrap();
            assert_eq!(measure["body_count"], 1);
            assert!((measure["bodies"][0]["volume_mm3"].as_f64().unwrap() - 6000.0).abs() < 1e-6);
        }
        session.execute("file.save", &json!({"path":packaged})).unwrap();
        assert_eq!(std::fs::read(solidcraft_io::backup_path(&packaged)).unwrap(), original);
        let before = std::fs::read(&packaged).unwrap();
        assert!(session.execute("file.save", &json!({"path":packaged,"format":"invalid"})).is_err());
        assert_eq!(std::fs::read(&packaged).unwrap(), before);
        session.execute("file.save", &json!({"path":packaged,"format":"json"})).unwrap();
        assert!(std::fs::read_to_string(&packaged).unwrap().contains("solidcraft/"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
