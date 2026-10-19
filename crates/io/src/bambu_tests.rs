use std::io::Read as _;

use solidcraft_geom::Vec3;

use super::*;
use crate::slicer::{DEFAULT_BAMBU_FILAMENTS, MAX_BAMBU_FILAMENTS, SlicerTarget, hex, rgba};

/// A cube of edge `s` with its lowest corner at `at` (12 triangles, outward).
fn cube(name: &str, at: Vec3, s: f64, color: Option<[f32; 3]>) -> MeshObject {
    let p = |x: f64, y: f64, z: f64| at + Vec3::new(x * s, y * s, z * s);
    let positions = vec![p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.), p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.)];
    let triangles =
        vec![[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
    MeshObject { name: name.into(), positions, triangles, color, ..Default::default() }
}

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.25, 1.0];
const GREEN: [f32; 3] = [0.0, 0.627, 0.0];
const YELLOW: [f32; 3] = [1.0, 1.0, 0.0];

/// Two red cubes, a blue one, a green one whose top face is yellow, and one with no colour.
fn five() -> Vec<MeshObject> {
    let mut green = cube("Green & <top>", Vec3::new(90.0, 0.0, 0.0), 20.0, Some(GREEN));
    green.looks = vec![(YELLOW, 0.0)];
    green.tri_looks = vec![0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    vec![
        cube("Red1", Vec3::ZERO, 20.0, Some(RED)),
        cube("Blue", Vec3::new(30.0, 0.0, 0.0), 20.0, Some(BLUE)),
        cube("Red2", Vec3::new(60.0, 0.0, 0.0), 20.0, Some(RED)),
        green,
        cube("Plain", Vec3::new(120.0, 0.0, -5.0), 20.0, None),
    ]
}

fn bambu(max: Option<usize>, face_colors: bool) -> SlicerOptions {
    SlicerOptions { target: SlicerTarget::Bambu, max_colors: max, face_colors, ..Default::default() }
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut x = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut x).unwrap();
    x
}

fn extruders(settings: &str) -> Vec<String> {
    settings.split("<metadata key=\"extruder\" value=\"").skip(1).map(|r| r.split('"').next().unwrap().to_string()).collect()
}

#[test]
fn paint_codes_follow_the_filament() {
    let codes: Vec<String> = (1..=32).map(|e| paint_code(e).unwrap()).collect();
    assert_eq!(&codes[..5], ["4", "8", "0C", "1C", "2C"]);
    // Checked by slicing: 16 DC, 17 EC, 18 0FC, 19 1FC, 20 2FC, 24 6FC, 31 DFC, 32 EFC.
    for (e, code) in [(16, "DC"), (17, "EC"), (18, "0FC"), (19, "1FC"), (20, "2FC"), (24, "6FC"), (31, "DFC"), (32, "EFC")] {
        assert_eq!(codes[e - 1], code, "filament {e}");
    }
    assert_eq!(paint_code(0), None);
    assert_eq!(paint_code(33), None);
}

/// Each colour a filament (in order of first use), each body on its colour's filament, the
/// yellow face painted with its own filament, and the package Bambu Studio reads as a project.
#[test]
fn project_has_a_filament_per_colour_and_painted_faces() {
    let out = write_bambu_3mf(&five(), &bambu(None, true), "Five", &[("Title", "Five <cubes>")]).unwrap();
    assert_eq!(out.colors.iter().map(|c| hex(c.color)).collect::<Vec<_>>(), ["#FF0000", "#0040FF", "#00A000", "#FFFF00", "#B3B3B3"]);
    assert_eq!(out.colors.iter().map(|c| c.triangles).collect::<Vec<_>>(), [24, 12, 10, 2, 12]);
    assert_eq!(out.colors.iter().map(|c| c.area.round()).collect::<Vec<_>>(), [4800.0, 2400.0, 2000.0, 400.0, 2400.0], "mm² of 20 mm cube faces");
    assert_eq!(out.extruders, [1, 2, 1, 3, 5]);
    assert_eq!((out.distinct_colors, out.merged_colors, out.painted_triangles), (5, 0, 2));
    assert!(out.mtl.is_none());

    let model = part(&out.bytes, "3D/3dmodel.model");
    assert!(model.contains(&format!("<metadata name=\"Application\">BambuStudio-{BAMBU_VERSION}</metadata>")), "{model}");
    assert!(model.contains("<metadata name=\"BambuStudio:3mfVersion\">1</metadata>"));
    assert!(model.contains("xmlns:BambuStudio=\"http://schemas.bambulab.com/package/2021\"") && model.contains("requiredextensions=\"p\""));
    assert!(model.contains("<metadata name=\"Title\">Five &lt;cubes&gt;</metadata>"));
    assert_eq!(model.matches("<component p:path=\"/3D/Objects/object_").count(), 5);
    assert_eq!(model.matches("<item objectid=").count(), 5);

    let settings = part(&out.bytes, "Metadata/model_settings.config");
    assert_eq!(extruders(&settings), ["1", "2", "1", "3", "5"]);
    assert!(settings.contains("value=\"Green &amp; &lt;top&gt;\""));

    let project: serde_json::Value = serde_json::from_str(&part(&out.bytes, "Metadata/project_settings.config")).unwrap();
    assert_eq!(project["filament_colour"], serde_json::json!(["#FF0000", "#0040FF", "#00A000", "#FFFF00", "#B3B3B3"]));
    // Every per-filament list has one entry per filament.
    for key in
        ["filament_density", "filament_diameter", "filament_ids", "filament_is_support", "filament_settings_id", "filament_shrink", "filament_type"]
    {
        assert_eq!(project[key].as_array().map(Vec::len), Some(5), "{key}");
    }
    assert_eq!(project["printer_settings_id"], "Bambu Lab P2S 0.4 nozzle");
    assert_eq!(project["filament_type"][0], "PLA");
    assert_eq!(project["nozzle_diameter"], serde_json::json!(["0.4"]));

    // Only the two yellow triangles of the green body are painted, with filament 4.
    let green = part(&out.bytes, "3D/Objects/object_4.model");
    assert_eq!(green.matches("paint_color=\"1C\"").count(), 2, "{green}");
    assert_eq!(green.matches("paint_color=").count(), 2);
    for k in [1, 2, 3, 5] {
        assert!(!part(&out.bytes, &format!("3D/Objects/object_{k}.model")).contains("paint_color"));
    }
    assert_eq!(out.bytes, write_bambu_3mf(&five(), &bambu(None, true), "Five", &[("Title", "Five <cubes>")]).unwrap().bytes, "reproducible bytes");
}

/// The package is a 3MF any reader opens (SolidCraft's own here): the bodies come back whole,
/// in their places relative to each other, centred on the plate and standing on it.
#[test]
#[ignore = "reads the project's model parts: needs the 3MF production-extension reader (#2089), which 0.16.2 lacks"]
fn project_reads_back_as_the_same_bodies_on_the_plate() {
    let out = write_bambu_3mf(&five(), &bambu(None, true), "Five", &[]).unwrap();
    let (objs, warnings) = crate::read_3mf(&out.bytes).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(objs.len(), 5);
    let volume = |o: &MeshObject| {
        let p = |i: u32| o.positions[i as usize];
        o.triangles.iter().map(|t| p(t[0]).dot(p(t[1]).cross(p(t[2])))).sum::<f64>() / 6.0
    };
    for o in &objs {
        assert!((volume(o) - 8000.0).abs() < 1e-3, "{}", volume(o));
    }
    let all: Vec<Vec3> = objs.iter().flat_map(|o| o.positions.iter().copied()).collect();
    let min = all.iter().fold(Vec3::new(f64::MAX, f64::MAX, f64::MAX), |a, p| Vec3::new(a.x.min(p.x), a.y.min(p.y), a.z.min(p.z)));
    let max = all.iter().fold(Vec3::new(f64::MIN, f64::MIN, f64::MIN), |a, p| Vec3::new(a.x.max(p.x), a.y.max(p.y), a.z.max(p.z)));
    assert!(((min.x + max.x) / 2.0 - 128.0).abs() < 1e-4 && ((min.y + max.y) / 2.0 - 128.0).abs() < 1e-4, "{min:?} {max:?}");
    assert!(min.z.abs() < 1e-4, "stands on the plate: {}", min.z);
    // The cube 5 mm lower than the others stays 5 mm lower.
    let low = objs[4].positions.iter().map(|p| p.z).fold(f64::MAX, f64::min);
    let red = objs[0].positions.iter().map(|p| p.z).fold(f64::MAX, f64::min);
    assert!((red - low - 5.0).abs() < 1e-4, "{red} {low}");
}

/// Over the limit the closest colours share filaments (the limit is 16 unless asked, 32 at most);
/// without face colours nothing is painted.
#[test]
fn colours_over_the_limit_join_the_nearest_and_face_colours_can_be_off() {
    let objs: Vec<MeshObject> =
        (0..20).map(|i| cube(&format!("C{i}"), Vec3::new(30.0 * i as f64, 0.0, 0.0), 10.0, Some([i as f32 / 19.0, 0.2, 0.4]))).collect();
    let out = write_bambu_3mf(&objs, &bambu(None, true), "Five", &[]).unwrap();
    assert_eq!(out.colors.len(), DEFAULT_BAMBU_FILAMENTS);
    assert_eq!((out.distinct_colors, out.merged_colors), (20, 4));
    assert_eq!(out.colors.iter().map(|c| c.sources.len()).sum::<usize>(), 20);
    assert!(out.extruders.iter().all(|e| (1..=16).contains(e)));
    let project: serde_json::Value = serde_json::from_str(&part(&out.bytes, "Metadata/project_settings.config")).unwrap();
    assert_eq!(project["filament_colour"].as_array().map(Vec::len), Some(16));
    // Up to 32 filaments; a limit asked above that is 32; one colour puts everything on filament 1.
    assert_eq!(write_bambu_3mf(&objs, &bambu(Some(20), true), "Five", &[]).unwrap().colors.len(), 20);
    let many: Vec<MeshObject> = (0..40)
        .map(|i| cube(&format!("C{i}"), Vec3::new(30.0 * i as f64, 0.0, 0.0), 10.0, Some([i as f32 / 39.0, 0.5, 1.0 - i as f32 / 39.0])))
        .collect();
    assert_eq!(write_bambu_3mf(&many, &bambu(Some(99), true), "Five", &[]).unwrap().colors.len(), MAX_BAMBU_FILAMENTS);
    let one = write_bambu_3mf(&five(), &bambu(Some(1), true), "Five", &[]).unwrap();
    assert_eq!(one.extruders, [1; 5]);
    assert_eq!((one.colors.len(), one.painted_triangles), (1, 0));
    assert_eq!(one.colors[0].sources.len(), 5, "every colour in the one filament");
    let flat = write_bambu_3mf(&five(), &bambu(None, false), "Five", &[]).unwrap();
    assert_eq!(flat.painted_triangles, 0);
    assert_eq!(flat.colors.len(), 4, "the yellow face takes its body's colour");
    assert!(!part(&flat.bytes, "3D/Objects/object_4.model").contains("paint_color"));
}

#[test]
fn presets_name_the_filament_type_and_nozzle() {
    assert_eq!(filament_type("Bambu PETG Basic @BBL P2S 0.4 nozzle"), "PETG");
    assert_eq!(filament_type("Generic ABS @BBL X1C"), "ABS");
    assert_eq!(filament_type("Something odd"), "PLA");
    assert_eq!(nozzle("Bambu Lab P2S 0.6 nozzle"), "0.6");
    assert_eq!(nozzle("My printer"), "0.4");
    assert_eq!(nozzle("Bambu Lab X1 NaN nozzle"), "0.4");
    let presets = BambuPresets { printer: "P \"x\"".into(), process: "Q".into(), filament: "Bambu PETG HF @BBL A1".into() };
    let options = SlicerOptions { presets, ..bambu(None, true) };
    let out = write_bambu_3mf(&five(), &options, "Five", &[]).unwrap();
    let project: serde_json::Value = serde_json::from_str(&part(&out.bytes, "Metadata/project_settings.config")).unwrap();
    assert_eq!(project["printer_settings_id"], "P \"x\"");
    assert_eq!(project["filament_type"][4], "PETG");
    assert_eq!(project["filament_density"][0], "1.27");
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    assert!(write_bambu_3mf(&[], &bambu(None, true), "Five", &[]).is_err());
    let mut nan = cube("NaN", Vec3::ZERO, 1.0, Some([f32::NAN, 0.0, 0.0]));
    nan.positions[0] = Vec3::new(f64::NAN, 0.0, 0.0);
    let empty = MeshObject { name: "Empty".into(), ..Default::default() };
    for objs in [vec![nan.clone()], vec![empty.clone()], vec![empty, nan]] {
        let r = std::panic::catch_unwind(|| write_bambu_3mf(&objs, &bambu(None, true), "Five", &[]).map(|o| o.bytes.len()));
        assert!(r.is_ok());
    }
    // Look entries that point past the list are the body's own colour.
    let mut odd = cube("Odd", Vec3::ZERO, 1.0, Some(RED));
    odd.tri_looks = vec![7; 12];
    let out = write_bambu_3mf(&[odd], &bambu(None, true), "Five", &[]).unwrap();
    assert_eq!((out.colors.len(), out.painted_triangles), (1, 0));
    assert_eq!(rgba(RED, 0.0), [255, 0, 0, 255]);
}

/// An assembly: one object with a part per body, each part at its place and on its filament,
/// the painted face kept; it reads back as the same bodies in the same places, as one mesh.
#[test]
fn assembly_is_one_object_with_a_part_per_body() {
    let options = SlicerOptions { assembly: true, ..bambu(None, true) };
    let out = write_bambu_3mf(&five(), &options, "Five cubes", &[]).unwrap();
    assert_eq!(out.extruders, [1, 2, 1, 3, 5]);
    assert_eq!(out.painted_triangles, 2);
    let model = part(&out.bytes, "3D/3dmodel.model");
    assert_eq!(model.matches("<object ").count(), 1, "{model}");
    assert!(model.contains("name=\"Five cubes\""));
    assert_eq!(model.matches("<component p:path=\"/3D/Objects/object_1.model\"").count(), 5);
    assert_eq!(model.matches("<item ").count(), 1);
    let settings = part(&out.bytes, "Metadata/model_settings.config");
    assert_eq!(settings.matches("<object ").count(), 1);
    assert_eq!(settings.matches("<part id=").count(), 5);
    assert_eq!(extruders(&settings), ["1", "2", "1", "3", "5"], "a filament per part");
    assert!(settings.contains("<metadata key=\"name\" value=\"Green &amp; &lt;top&gt;\"/>"));
    let meshes = part(&out.bytes, "3D/Objects/object_1.model");
    assert_eq!(meshes.matches("<mesh>").count(), 5);
    assert_eq!(meshes.matches("paint_color=\"1C\"").count(), 2);
    let mut zip = zip::ZipArchive::new(Cursor::new(out.bytes.as_slice())).unwrap();
    assert!(zip.by_name("3D/Objects/object_2.model").is_err(), "all parts in one model part");
    // The parts' places: each component's translation is its body's centre, shifted as one.
    let centres: Vec<[f64; 3]> = model
        .split("transform=\"1 0 0 0 1 0 0 0 1 ")
        .skip(1)
        .take(5)
        .map(|t| {
            let v: Vec<f64> = t.split('"').next().unwrap().split(' ').map(|x| x.parse().unwrap()).collect();
            [v[0], v[1], v[2]]
        })
        .collect();
    let want = [[10.0, 10.0, 10.0], [40.0, 10.0, 10.0], [70.0, 10.0, 10.0], [100.0, 10.0, 10.0], [130.0, 10.0, 5.0]];
    let d = [centres[0][0] - want[0][0], centres[0][1] - want[0][1], centres[0][2] - want[0][2]];
    for (c, w) in centres.iter().zip(want) {
        for i in 0..3 {
            assert!((c[i] - w[i] - d[i]).abs() < 1e-4, "{centres:?}");
        }
    }
    let (objs, warnings) = crate::read_3mf(&out.bytes).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(objs.len(), 1);
    assert_eq!(objs[0].name, "Five cubes");
    assert_eq!(objs[0].triangles.len(), 60);
}

/// Faces in up to 32 colours on one body: every face colour its own filament, painted with
/// codes up to `EFC`.
#[test]
fn faces_paint_up_to_filament_32() {
    let mut body = cube("Many", Vec3::ZERO, 10.0, Some([0.5, 0.5, 0.5]));
    let mut boxes = vec![body.clone()];
    // 31 more bodies, each a colour of its own, so that the faces of the first can use them all.
    for i in 1..32 {
        boxes.push(cube(&format!("B{i}"), Vec3::new(20.0 * i as f64, 0.0, 0.0), 10.0, Some([i as f32 / 31.0, 1.0 - i as f32 / 31.0, 0.3])));
    }
    body.looks = vec![([1.0 / 31.0 * 31.0, 0.0, 0.3], 0.0)];
    body.tri_looks = vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    boxes[0] = body;
    let out = write_bambu_3mf(&boxes, &bambu(Some(32), true), "Many", &[]).unwrap();
    assert_eq!(out.colors.len(), 32);
    let first = part(&out.bytes, "3D/Objects/object_1.model");
    let code = first.split("paint_color=\"").nth(1).and_then(|r| r.split('"').next()).unwrap().to_string();
    let filament = out.colors.iter().position(|c| c.color == rgba([1.0, 0.0, 0.3], 0.0)).unwrap() + 1;
    assert_eq!(Some(code), paint_code(filament), "the painted face's filament {filament}");
}
