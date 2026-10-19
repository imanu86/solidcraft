//! Mesh exports made for a slicer (docs/formats/slicer-export.md): `slicer_compat`, `mtl`,
//! `max_colors`, `face_colors`, the Bambu Studio presets and `visible_only` on `file.export` and
//! `file.save_mesh`, and the plain exports they leave unchanged.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solidcraft-slicer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn at(d: &std::path::Path, file: &str) -> String {
    d.join(file).to_string_lossy().to_string()
}

/// Three 20 mm cubes in a row, red, blue and red, the blue one's top face yellow; and a grey
/// one far off, hidden.
fn cubes() -> Session {
    let mut s = Session::default();
    for (k, name) in ["Red1", "Blue", "Red2", "Hidden"].iter().enumerate() {
        let x = if *name == "Hidden" { 200.0 } else { 30.0 * k as f64 };
        run(&mut s, "solid.box", json!({"length": 20, "width": 20, "height": 20, "corner": [x, 0, 0], "body_name": name, "operation": "new"}));
    }
    run(&mut s, "appearance.assign", json!({"bodies": ["Red1", "Red2"], "color": "#ff0000"}));
    run(&mut s, "appearance.assign", json!({"bodies": ["Blue"], "color": "#0040ff"}));
    run(&mut s, "appearance.assign", json!({"faces": [[40, 10, 20]], "color": "#ffff00"}));
    run(&mut s, "browser.visibility", json!({"visible": false, "items": ["Hidden"]}));
    s
}

/// The names of the bodies in a 3MF, as SolidCraft's own reader sees them.
fn names(path: &str) -> Vec<String> {
    let (objs, _) = solidcraft_io::read_3mf(&std::fs::read(path).unwrap()).unwrap();
    objs.into_iter().map(|o| o.name).collect()
}

/// No slicer options: the file is the plain export, byte for byte, whichever way it is asked for.
#[test]
fn without_slicer_options_exports_are_unchanged() {
    let mut s = cubes();
    let d = dir("plain");
    for sub in ["plain", "none", "mesh"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    // The same file name each time: STL and STEP name their contents after it.
    for ext in ["3mf", "obj", "stl", "step"] {
        let file = |sub: &str| at(&d.join(sub), &format!("cubes.{ext}"));
        run(&mut s, "file.export", json!({"path": file("plain")}));
        let r = run(&mut s, "file.export", json!({"path": file("none"), "slicer_compat": "none", "mtl": false, "visible_only": false}));
        assert!(r.get("slicer_compat").is_none(), "{r}");
        let bytes = std::fs::read(file("plain")).unwrap();
        assert!(bytes == std::fs::read(file("none")).unwrap(), "{ext}");
        if ext != "step" {
            run(&mut s, "file.save_mesh", json!({"path": file("mesh")}));
            assert!(bytes == std::fs::read(file("mesh")).unwrap(), "{ext}");
        }
    }
    assert!(!d.join("plain").join("cubes.mtl").exists());
    // The plain 3MF holds every body, hidden ones too, as it always did.
    assert_eq!(names(&at(&d.join("plain"), "cubes.3mf")), ["Red1", "Blue", "Red2", "Hidden"]);
    let _ = std::fs::remove_dir_all(&d);
}

/// A Bambu Studio project: a filament per colour, each body on its colour's filament, the yellow
/// face painted, the hidden body left out on request.
#[test]
fn bambu_project_has_filaments_extruders_and_painted_faces() {
    let mut s = cubes();
    let d = dir("bambu");
    let path = at(&d, "cubes.3mf");
    let r = run(
        &mut s,
        "file.export",
        json!({"path": path, "slicer_compat": "bambu", "visible_only": true, "filament": "Bambu PETG Basic @BBL P2S 0.4 nozzle"}),
    );
    assert_eq!(r["slicer_compat"], "bambu");
    assert_eq!(r["bodies"], 3);
    assert_eq!(
        r["filaments"],
        json!([
            {"color": "#FF0000", "triangles": 24, "area_mm2": 4800.0, "merged": 1, "sources": ["#FF0000"]},
            {"color": "#0040FF", "triangles": 10, "area_mm2": 2000.0, "merged": 1, "sources": ["#0040FF"]},
            {"color": "#FFFF00", "triangles": 2, "area_mm2": 400.0, "merged": 1, "sources": ["#FFFF00"]}
        ])
    );
    assert_eq!(r["painted_triangles"], 2);
    assert_eq!(r["extruders"], json!([1, 2, 1]));
    // (0.16.2 cannot read a project's model parts back: #2089.)
    // Without visible_only the hidden body goes too, on its own (default grey) filament.
    let all = run(&mut s, "file.export", json!({"path": path, "slicer_compat": "bambu", "face_colors": false}));
    assert_eq!(all["bodies"], 4);
    assert_eq!(all["painted_triangles"], 0);
    assert_eq!(all["filaments"].as_array().map(Vec::len), Some(3), "red, blue, grey: the yellow face is its body's blue");
    // As one assembly: one object (named after the file) with a part per body, each on its
    // own filament; the bodies read back as one mesh holding all of them.
    let asm = run(&mut s, "file.export", json!({"path": path, "slicer_compat": "bambu", "visible_only": true, "assembly": true}));
    assert_eq!(asm["extruders"], json!([1, 2, 1]));
    assert_eq!(asm["painted_triangles"], 2);
    assert_eq!(names(&path), ["cubes"]);
    // One colour: every body on filament 1.
    let one = run(&mut s, "file.export", json!({"path": path, "slicer_compat": "bambu", "max_colors": 1}));
    assert_eq!(one["filaments"].as_array().map(Vec::len), Some(1));
    assert_eq!(one["merged_colors"], 3);
    assert_eq!(one["filaments"][0]["merged"], 4);
    // Up to 32 filaments.
    assert!(s.execute("file.export", &json!({"path": path, "slicer_compat": "bambu", "max_colors": 32})).is_ok());
    let _ = std::fs::remove_dir_all(&d);
}

/// OBJ for Bambu Studio, or any OBJ with `mtl`: the MTL file next to it, named in the OBJ; and
/// Save as mesh takes the same options.
#[test]
fn obj_with_mtl_writes_both_files() {
    let mut s = cubes();
    let d = dir("obj");
    let path = at(&d, "cubes.obj");
    let r = run(&mut s, "file.export", json!({"path": path, "mtl": true, "visible_only": true}));
    assert_eq!(r["files"], json!([at(&d, "cubes.mtl"), path]));
    assert_eq!(r["materials"].as_array().map(Vec::len), Some(3));
    let obj = std::fs::read_to_string(&path).unwrap();
    assert!(obj.contains("\nmtllib cubes.mtl\n") && obj.contains("usemtl solidcraft_FFFF00"), "{}", &obj[..200]);
    let mtl = std::fs::read_to_string(d.join("cubes.mtl")).unwrap();
    assert_eq!(mtl.matches("newmtl ").count(), 3);
    let saved = at(&d, "saved.obj");
    let r = run(&mut s, "file.save_mesh", json!({"path": saved, "slicer_compat": "bambu", "max_colors": 2, "bodies": ["Red1", "Blue"]}));
    assert_eq!(r["materials"].as_array().map(Vec::len), Some(2));
    assert!(d.join("saved.mtl").exists());
    // As one assembly: one object named after the file, a group per body.
    run(&mut s, "file.export", json!({"path": path, "mtl": true, "visible_only": true, "assembly": true}));
    let obj = std::fs::read_to_string(&path).unwrap();
    assert_eq!(obj.lines().filter(|l| l.starts_with("o ")).collect::<Vec<_>>(), ["o cubes"]);
    assert_eq!(obj.lines().filter(|l| l.starts_with("g ")).count(), 3);
    let _ = std::fs::remove_dir_all(&d);
}

/// Options that make no sense for the export are refused before anything is written.
#[test]
fn slicer_options_are_checked() {
    let mut s = cubes();
    let d = dir("bad");
    let p3 = at(&d, "x.3mf");
    let cases = [
        json!({"path": p3, "slicer_compat": "cura"}),
        json!({"path": p3, "slicer_compat": "bambu", "max_colors": 0}),
        json!({"path": p3, "slicer_compat": "bambu", "max_colors": 33}),
        json!({"path": p3, "slicer_compat": "bambu", "max_colors": 2.5}),
        json!({"path": p3, "slicer_compat": "bambu", "face_colors": "yes"}),
        json!({"path": p3, "slicer_compat": "bambu", "printer": ""}),
        json!({"path": p3, "slicer_compat": "bambu", "printer": "x".repeat(300)}),
        json!({"path": p3, "max_colors": 4}),
        json!({"path": p3, "printer": "Bambu Lab X1 Carbon 0.4 nozzle"}),
        json!({"path": p3, "mtl": true}),
        json!({"path": p3, "assembly": true}),
        json!({"path": p3, "slicer_compat": "bambu", "assembly": 1}),
        json!({"path": p3, "format": "3mf-tagged", "slicer_compat": "bambu"}),
        json!({"path": at(&d, "x.stl"), "slicer_compat": "bambu"}),
        json!({"path": at(&d, "x.step"), "slicer_compat": "bambu"}),
        json!({"path": at(&d, "x.obj"), "slicer_compat": "bambu", "filament": "Bambu PLA Basic @BBL P2S"}),
        json!({"path": at(&d, "x.urdf"), "visible_only": true}),
        json!({"path": p3, "visible_only": "true"}),
        json!({"path": p3, "visible_only": true, "bodies": ["Hidden"]}),
    ];
    for p in cases {
        assert!(s.execute("file.export", &p).is_err(), "{p}");
    }
    assert!(s.execute("file.save_mesh", &json!({"path": at(&d, "x.stl"), "slicer_compat": "bambu"})).is_err());
    assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "nothing written");
    let _ = std::fs::remove_dir_all(&d);
}
