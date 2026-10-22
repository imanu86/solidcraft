//! Wavefront OBJ with an MTL file, for slicers: each body welded watertight as the 3MF export
//! makes it (single-precision coordinates, no normals), one material per colour, and a `usemtl`
//! wherever the colour of the faces changes. Slicers that read OBJ colours (Bambu Studio among
//! them) take one filament per material. The plain OBJ export ([`crate::obj()`]) is unchanged.

use std::fmt::Write as _;

use crate::slicer::{DEFAULT_COLOR, Rgba, SlicerExport, SlicerOptions, SlicerTarget, Usage, area, rgba};
use crate::threemf::{MeshObject, indexed};

/// The material name of a colour (`solidcraft_FF8000`, with the alpha when see-through).
fn material(c: Rgba) -> String {
    if c[3] == 255 {
        format!("solidcraft_{:02X}{:02X}{:02X}", c[0], c[1], c[2])
    } else {
        format!("solidcraft_{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3])
    }
}

/// The OBJ of `objects` (one `o` per body, or with `options.assembly` one `o` named `name` and a
/// `g` per body; naming the MTL file `mtl_name`) and its MTL file, with at most `options`' limit
/// of colours.
pub(crate) fn obj_with_mtl(objects: &[MeshObject], options: &SlicerOptions, name: &str, mtl_name: &str) -> SlicerExport {
    // The colour of every triangle. A slicer's filaments have no transparency: for Bambu Studio
    // colours that differ only in it share a material.
    let see_through = options.target != SlicerTarget::Bambu;
    let look = |c: [f32; 3], transparency: f32| rgba(c, if see_through { transparency } else { 0.0 });
    let mut usage = Usage::default();
    let mut shapes = Vec::with_capacity(objects.len());
    for o in objects {
        let (verts, tris) = indexed(o);
        let own = look(o.color.unwrap_or(DEFAULT_COLOR), o.transparency);
        let colors: Vec<Rgba> = tris
            .iter()
            .map(|t| match t.look.checked_sub(1).and_then(|i| o.looks.get(i as usize)) {
                Some((c, transparency)) if options.face_colors => look(*c, *transparency),
                _ => own,
            })
            .collect();
        for (c, t) in colors.iter().zip(&tris) {
            usage.add(*c, area(&verts, t.v));
        }
        shapes.push((verts, tris, colors));
    }
    let palette = usage.palette(options.limit());
    let mut mtl = String::from("# SolidCraft OBJ materials: one per colour\n");
    for c in &palette.colors() {
        let [r, g, b, a] = c.map(|v| f32::from(v) / 255.0);
        let _ = write!(mtl, "\nnewmtl {}\nKa {r:.4} {g:.4} {b:.4}\nKd {r:.4} {g:.4} {b:.4}\nKs 0 0 0\nNs 10\nd {a:.4}\nillum 1\n", material(*c));
    }
    let mut s = String::from("# SolidCraft OBJ export (mm)\n");
    let _ = writeln!(s, "mtllib {mtl_name}");
    let mut base = 1usize;
    if options.assembly {
        let _ = writeln!(s, "o {}", name.replace(char::is_whitespace, "_"));
    }
    for (o, (verts, tris, colors)) in objects.iter().zip(&shapes) {
        let _ = writeln!(s, "{} {}", if options.assembly { "g" } else { "o" }, o.name.replace(char::is_whitespace, "_"));
        for v in verts {
            let _ = writeln!(s, "v {} {} {}", v[0] as f32, v[1] as f32, v[2] as f32);
        }
        let mut current = None;
        for (t, c) in tris.iter().zip(colors) {
            let slot = palette.slot(*c);
            if current != Some(slot) {
                if let Some(k) = palette.slots.get(slot) {
                    let _ = writeln!(s, "usemtl {}", material(k.color));
                }
                current = Some(slot);
            }
            let [a, b, c] = t.v.map(|i| i as usize + base);
            let _ = writeln!(s, "f {a} {b} {c}");
        }
        base += verts.len();
    }
    SlicerExport {
        bytes: s.into_bytes(),
        mtl: Some(mtl.into_bytes()),
        colors: palette.slots.clone(),
        distinct_colors: usage.distinct(),
        merged_colors: palette.merged(),
        painted_triangles: 0,
        extruders: Vec::new(),
        bodies: objects.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_geom::Vec3;

    fn tetra(name: &str, at: f64, color: Option<[f32; 3]>) -> MeshObject {
        MeshObject {
            name: name.into(),
            positions: vec![Vec3::new(at, 0.0, 0.0), Vec3::new(at + 10.0, 0.0, 0.0), Vec3::new(at, 10.0, 0.0), Vec3::new(at, 0.0, 10.0)],
            triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
            color,
            ..Default::default()
        }
    }

    fn mtl_options(max: Option<usize>, face_colors: bool) -> SlicerOptions {
        SlicerOptions { mtl: true, max_colors: max, face_colors, ..Default::default() }
    }

    /// One material per colour, `usemtl` where the colour changes, faces keeping their own
    /// colour, and the OBJ reads back (SolidCraft's own reader) as the same bodies.
    #[test]
    fn obj_names_a_material_per_colour() {
        let mut b = tetra("Blue part", 20.0, Some([0.0, 0.25, 1.0]));
        b.looks = vec![([1.0, 1.0, 0.0], 0.5)];
        b.tri_looks = vec![0, 0, 1, 0];
        let objs = [tetra("Red", 0.0, Some([1.0, 0.0, 0.0])), b, tetra("Plain", 40.0, None)];
        let out = obj_with_mtl(&objs, &mtl_options(None, true), "parts", "parts.mtl");
        let obj = String::from_utf8(out.bytes.clone()).unwrap();
        let mtl = String::from_utf8(out.mtl.clone().unwrap()).unwrap();
        assert!(obj.starts_with("# SolidCraft OBJ export (mm)\nmtllib parts.mtl\no Red\n"), "{obj}");
        assert!(obj.contains("o Blue_part\n"));
        assert_eq!(
            obj.lines().filter(|l| l.starts_with("usemtl ")).collect::<Vec<_>>(),
            [
                "usemtl solidcraft_FF0000",
                "usemtl solidcraft_0040FF",
                "usemtl solidcraft_FFFF0080",
                "usemtl solidcraft_0040FF",
                "usemtl solidcraft_B3B3B3"
            ]
        );
        assert_eq!(mtl.matches("newmtl ").count(), 4);
        assert!(mtl.contains("newmtl solidcraft_FFFF0080\nKa 1.0000 1.0000 0.0000\nKd 1.0000 1.0000 0.0000\nKs 0 0 0\nNs 10\nd 0.5020\n"), "{mtl}");
        assert_eq!(out.colors.iter().map(|c| c.triangles).sum::<usize>(), 12);
        let (back, warnings) = crate::read_obj(&out.bytes, "x").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.iter().map(|o| (o.name.as_str(), o.triangles.len())).collect::<Vec<_>>(), [("Red", 4), ("Blue_part", 4), ("Plain", 4)]);
        assert!(!obj.contains("vn "), "no normals: the slicer works them out");
    }

    /// The limit merges the least used colours into the nearest; without face colours each body
    /// is its own colour; for Bambu Studio see-through colours are written opaque.
    #[test]
    fn obj_colours_follow_the_options() {
        let mut b = tetra("B", 20.0, Some([0.0, 0.25, 1.0]));
        b.looks = vec![([1.0, 1.0, 0.0], 0.0)];
        b.tri_looks = vec![0, 0, 1, 0];
        b.transparency = 0.25;
        let objs = [tetra("A", 0.0, Some([1.0, 0.0, 0.0])), tetra("A2", 10.0, Some([0.9, 0.0, 0.0])), b];
        let two = obj_with_mtl(&objs, &mtl_options(Some(2), true), "a", "a.mtl");
        assert_eq!(two.colors.len(), 2);
        assert_eq!(two.merged_colors, 2);
        let flat = obj_with_mtl(&objs, &mtl_options(None, false), "a", "a.mtl");
        assert_eq!(flat.colors.len(), 3);
        let opaque = obj_with_mtl(&objs, &SlicerOptions { target: SlicerTarget::Bambu, ..mtl_options(None, true) }, "a", "a.mtl");
        let mtl = String::from_utf8(opaque.mtl.unwrap()).unwrap();
        assert!(mtl.contains("newmtl solidcraft_0040FF\n") && !mtl.contains("d 0.7"), "{mtl}");
    }

    /// An assembly: one `o`, a `g` per body; the bodies are the same.
    #[test]
    fn obj_assembly_is_one_object_with_a_group_per_body() {
        let objs = [tetra("Red part", 0.0, Some([1.0, 0.0, 0.0])), tetra("Blue", 20.0, Some([0.0, 0.25, 1.0]))];
        let out = obj_with_mtl(&objs, &SlicerOptions { assembly: true, ..mtl_options(None, true) }, "Whole frame", "w.mtl");
        let obj = String::from_utf8(out.bytes.clone()).unwrap();
        assert!(obj.starts_with("# SolidCraft OBJ export (mm)\nmtllib w.mtl\no Whole_frame\ng Red_part\n"), "{obj}");
        assert_eq!(obj.lines().filter(|l| l.starts_with("o ")).count(), 1);
        assert_eq!(obj.lines().filter(|l| l.starts_with("g ")).count(), 2);
        let (back, _) = crate::read_obj(&out.bytes, "x").unwrap();
        assert_eq!(back.iter().map(|o| o.triangles.len()).sum::<usize>(), 8);
    }
}
