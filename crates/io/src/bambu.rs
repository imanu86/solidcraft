//! Bambu Studio project 3MF: each body an object with a filament of its own colour, and the
//! triangles of faces in another colour painted with that colour's filament.
//!
//! The package was worked out from the files Bambu Studio itself writes and reads back, by trying
//! packages on its command-line program (load, export again, slice) and looking at the results; it
//! was not taken from Bambu Studio's sources. What the program needs:
//!
//! - `3D/3dmodel.model`: the 3MF core and production namespaces plus Bambu Studio's
//!   (`requiredextensions="p"`), the metadata `Application` = `BambuStudio-<version>` (other
//!   applications' files are read as geometry only) and `BambuStudio:3mfVersion` = 1, and per body
//!   an object made of one component in a model part of its own, `3D/Objects/object_<n>.model`,
//!   placed by a build item.
//! - `Metadata/model_settings.config`: per object its name and `extruder` (its filament, from 1).
//! - `Metadata/project_settings.config`: a JSON object with the filaments (`filament_colour` and
//!   the other per-filament lists, one entry per filament) and the printer, process and filament
//!   presets by name. Without the names, the nozzle, the printable height or a per-filament list
//!   (`filament_ids` among them, even empty) the program does not load the project, or fails when
//!   it saves it sliced; the settings left out take Bambu Studio's own values.
//! - A triangle painted with filament `e` carries `paint_color`: `4` for 1, `8` for 2, and from 3
//!   on the hexadecimal digit of `e - 3` followed by `C` (`0C` … `DC` for 16).
//!
//! Mixed filaments (Bambu Studio 2.5.3 and later) are not written: their settings are not known
//! yet. [`FilamentSlot`] is where such a slot would go.

use std::fmt::Write as _;
use std::io::{Cursor, Write};

use crate::slicer::{BambuPresets, DEFAULT_COLOR, Rgba, SlicerExport, SlicerOptions, Usage, hex, rgba};
use crate::threemf::{MeshObject, esc, indexed};
use crate::{IoError, Result};

/// The Bambu Studio file version the package is written as (the version it was checked with).
pub const BAMBU_VERSION: &str = "02.08.02.61";
const CORE_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const PRODUCTION_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
const BAMBU_NS: &str = "http://schemas.bambulab.com/package/2021";
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
/// Where the model's centre goes on the plate (the middle of a 256 mm plate); Bambu Studio's
/// Arrange moves it for other plates.
const PLATE_CENTRE: [f64; 2] = [128.0, 128.0];
/// The printable height written for the printer (that of 256 mm printers); the printer preset
/// chosen in Bambu Studio has its own.
const PRINTABLE_HEIGHT: &str = "256";

/// One filament of the project. A plain colour now; a mixed filament (several filaments in
/// ratios, Bambu Studio 2.5.3 and later) would be another kind of slot.
#[derive(Clone, Debug, PartialEq)]
pub struct FilamentSlot {
    pub color: Rgba,
}

/// Bambu Studio's `paint_color` for a whole triangle painted with filament `e` (from 1).
pub fn paint_code(e: usize) -> Option<String> {
    match e {
        1 => Some("4".into()),
        2 => Some("8".into()),
        3..=18 => Some(format!("{:X}C", e - 3)),
        _ => None,
    }
}

/// A body ready to write: vertices about its own centre, its placement, its triangles and their
/// filaments.
struct Part {
    name: String,
    vertices: Vec<[f64; 3]>,
    centre: [f64; 3],
    triangles: Vec<[u32; 3]>,
    /// Filament (from 1) of each triangle.
    filaments: Vec<usize>,
    extruder: usize,
}

/// A Bambu Studio project of `objects` (one per body): one filament per colour, up to the limit
/// of `options` (16 at most), each body on the filament most of it uses, and with
/// `options.face_colors` the triangles of other colours painted. `metadata` goes into the model
/// (3MF names such as `Title`).
pub fn write_bambu_3mf(objects: &[MeshObject], options: &SlicerOptions, metadata: &[(&str, &str)]) -> Result<SlicerExport> {
    if objects.is_empty() {
        return Err(IoError::Empty);
    }
    // The colour of every triangle, and how much each colour is used.
    let mut usage = Usage::default();
    let mut shapes = Vec::with_capacity(objects.len());
    for o in objects {
        let (verts, tris) = indexed(o);
        let own = rgba(o.color.unwrap_or(DEFAULT_COLOR), 0.0);
        let colors: Vec<Rgba> = tris
            .iter()
            .map(|t| match t.look.checked_sub(1).and_then(|i| o.looks.get(i as usize)) {
                Some((c, _)) if options.face_colors => rgba(*c, 0.0),
                _ => own,
            })
            .collect();
        for c in &colors {
            usage.add(*c, 1);
        }
        shapes.push((o.name.clone(), verts, tris.into_iter().map(|t| t.v).collect::<Vec<_>>(), colors));
    }
    let palette = usage.palette(options.limit());
    let slots: Vec<FilamentSlot> = palette.colors.iter().map(|c| FilamentSlot { color: *c }).collect();
    // Placement: the model's centre over the plate's, standing on it.
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (_, verts, _, _) in &shapes {
        for v in verts {
            for i in 0..3 {
                lo[i] = lo[i].min(v[i]);
                hi[i] = hi[i].max(v[i]);
            }
        }
    }
    if !lo.iter().chain(&hi).all(|x| x.is_finite()) {
        return Err(IoError::Empty);
    }
    let shift = [PLATE_CENTRE[0] - (lo[0] + hi[0]) / 2.0, PLATE_CENTRE[1] - (lo[1] + hi[1]) / 2.0, -lo[2]];
    let mut parts = Vec::with_capacity(shapes.len());
    let mut painted = 0usize;
    for (name, verts, triangles, colors) in shapes {
        let filaments: Vec<usize> = colors.iter().map(|c| palette.slot(*c) + 1).collect();
        let mut count = vec![0usize; slots.len() + 1];
        for f in &filaments {
            if let Some(n) = count.get_mut(*f) {
                *n += 1;
            }
        }
        // The filament most of the body uses (the first of equals).
        let extruder = count.iter().enumerate().skip(1).max_by_key(|(i, n)| (**n, std::cmp::Reverse(*i))).map_or(1, |(i, _)| i);
        painted += filaments.iter().filter(|f| **f != extruder).count();
        let (mut a, mut b) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in &verts {
            for i in 0..3 {
                a[i] = a[i].min(v[i]);
                b[i] = b[i].max(v[i]);
            }
        }
        let centre = if verts.is_empty() { [0.0; 3] } else { [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0] };
        let vertices = verts.iter().map(|v| [v[0] - centre[0], v[1] - centre[1], v[2] - centre[2]]).collect();
        let centre = [centre[0] + shift[0], centre[1] + shift[1], centre[2] + shift[2]];
        parts.push(Part { name, vertices, centre, triangles, filaments, extruder });
    }
    let bytes = package(&parts, &slots, &options.presets, metadata, options.face_colors)?;
    Ok(SlicerExport {
        bytes,
        mtl: None,
        colors: palette.colors.iter().zip(&palette.triangles).map(|(c, n)| (hex(*c), *n)).collect(),
        distinct_colors: usage.distinct(),
        merged_colors: palette.merged,
        painted_triangles: if options.face_colors { painted } else { 0 },
        extruders: parts.iter().map(|p| p.extruder).collect(),
        bodies: parts.len(),
    })
}

/// A UUID for element `index` of kind `kind` (fixed per file, so exports are reproducible).
fn uuid(kind: u32, index: usize) -> String {
    format!("{kind:08x}-50cd-4c4a-8f00-{:012x}", index as u64)
}

const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

fn model_open(x: &mut String) {
    x.push_str(XML_HEAD);
    let _ = writeln!(
        x,
        "<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"{CORE_NS}\" xmlns:BambuStudio=\"{BAMBU_NS}\" xmlns:p=\"{PRODUCTION_NS}\" requiredextensions=\"p\">"
    );
}

/// `3D/Objects/object_<n>.model`: one body's mesh.
fn object_part(p: &Part, id: usize, index: usize, paint: bool) -> String {
    let mut x = String::with_capacity(64 + p.vertices.len() * 48 + p.triangles.len() * 48);
    model_open(&mut x);
    x.push_str(" <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n <resources>\n");
    let _ = writeln!(x, "  <object id=\"{id}\" p:UUID=\"{}\" type=\"model\">", uuid(1, index));
    x.push_str("   <mesh>\n    <vertices>\n");
    for v in &p.vertices {
        let _ = writeln!(x, "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", v[0] as f32, v[1] as f32, v[2] as f32);
    }
    x.push_str("    </vertices>\n    <triangles>\n");
    for (t, f) in p.triangles.iter().zip(&p.filaments) {
        let _ = write!(x, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"", t[0], t[1], t[2]);
        if paint
            && *f != p.extruder
            && let Some(code) = paint_code(*f)
        {
            let _ = write!(x, " paint_color=\"{code}\"");
        }
        x.push_str("/>\n");
    }
    x.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n <build/>\n</model>\n");
    x
}

/// `3D/3dmodel.model`: an object per body placing its part, and a build item per object.
fn root_model(parts: &[Part], metadata: &[(&str, &str)]) -> String {
    let mut x = String::new();
    model_open(&mut x);
    let _ = writeln!(x, " <metadata name=\"Application\">BambuStudio-{BAMBU_VERSION}</metadata>");
    x.push_str(" <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n");
    for (name, text) in metadata {
        let _ = writeln!(x, " <metadata name=\"{}\">{}</metadata>", esc(name), esc(text));
    }
    x.push_str(" <resources>\n");
    for (k, p) in parts.iter().enumerate() {
        let _ = writeln!(x, "  <object id=\"{}\" p:UUID=\"{}\" type=\"model\" name=\"{}\">\n   <components>", 2 * k + 2, uuid(2, k), esc(&p.name));
        let _ = writeln!(
            x,
            "    <component p:path=\"/3D/Objects/object_{}.model\" objectid=\"{}\" p:UUID=\"{}\" transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/>",
            k + 1,
            2 * k + 1,
            uuid(3, k)
        );
        x.push_str("   </components>\n  </object>\n");
    }
    let _ = writeln!(x, " </resources>\n <build p:UUID=\"{}\">", uuid(4, 0));
    for (k, p) in parts.iter().enumerate() {
        let c = p.centre;
        let _ = writeln!(
            x,
            "  <item objectid=\"{}\" p:UUID=\"{}\" transform=\"1 0 0 0 1 0 0 0 1 {} {} {}\" printable=\"1\"/>",
            2 * k + 2,
            uuid(5, k),
            c[0] as f32,
            c[1] as f32,
            c[2] as f32
        );
    }
    x.push_str(" </build>\n</model>\n");
    x
}

/// `Metadata/model_settings.config`: each object's name and filament.
fn model_settings(parts: &[Part]) -> String {
    let mut x = String::from(XML_HEAD);
    x.push_str("<config>\n");
    for (k, p) in parts.iter().enumerate() {
        let name = esc(&p.name);
        let _ = writeln!(x, "  <object id=\"{}\">", 2 * k + 2);
        let _ = writeln!(x, "    <metadata key=\"name\" value=\"{name}\"/>");
        let _ = writeln!(x, "    <metadata key=\"extruder\" value=\"{}\"/>", p.extruder);
        let _ = writeln!(x, "    <part id=\"{}\" subtype=\"normal_part\">", 2 * k + 1);
        let _ = writeln!(x, "      <metadata key=\"name\" value=\"{name}\"/>");
        x.push_str("      <metadata key=\"matrix\" value=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/>\n    </part>\n  </object>\n");
    }
    x.push_str("  <plate>\n    <metadata key=\"plater_id\" value=\"1\"/>\n    <metadata key=\"plater_name\" value=\"\"/>\n    <metadata key=\"locked\" value=\"false\"/>\n  </plate>\n</config>\n");
    x
}

/// The material type a filament preset name says (`Bambu PETG Basic …` → `PETG`), else `PLA`.
fn filament_type(preset: &str) -> &'static str {
    const TYPES: [&str; 14] = ["PLA", "PETG", "PCTG", "PET", "ABS", "ASA", "TPU", "PA", "PC", "PVA", "HIPS", "PP", "PPS", "PE"];
    preset
        .split(|c: char| c.is_whitespace() || c == '-' || c == '@')
        .find_map(|w| TYPES.iter().find(|t| w.eq_ignore_ascii_case(t)).copied())
        .unwrap_or("PLA")
}

/// A typical density of a material type, g/cm³ (only for the weights Bambu Studio estimates
/// before its own filament preset is applied).
fn density(kind: &str) -> &'static str {
    match kind {
        "PETG" => "1.27",
        "PCTG" | "PVA" => "1.23",
        "PET" => "1.38",
        "ABS" | "HIPS" => "1.04",
        "ASA" => "1.07",
        "TPU" => "1.21",
        "PA" => "1.14",
        "PC" => "1.20",
        "PP" => "0.90",
        "PPS" => "1.35",
        "PE" => "0.95",
        _ => "1.24",
    }
}

/// The nozzle a printer preset name says (`… 0.6 nozzle` → `0.6`), else `0.4`.
fn nozzle(printer: &str) -> String {
    let words: Vec<&str> = printer.split_whitespace().collect();
    words
        .windows(2)
        .find(|w| w.get(1).is_some_and(|n| n.eq_ignore_ascii_case("nozzle")))
        .and_then(|w| w.first())
        .filter(|d| d.parse::<f64>().is_ok_and(|v| v > 0.0 && v < 5.0))
        .map_or_else(|| "0.4".to_string(), |d| (*d).to_string())
}

/// `Metadata/project_settings.config`: the filaments and the presets by name.
fn project_settings(slots: &[FilamentSlot], presets: &BambuPresets) -> String {
    let n = slots.len();
    let kind = filament_type(&presets.filament);
    let each = |v: &str| serde_json::Value::from(vec![v; n]);
    let config = serde_json::json!({
        "filament_colour": slots.iter().map(|s| hex(s.color)).collect::<Vec<_>>(),
        "filament_density": each(density(kind)),
        "filament_diameter": each("1.75"),
        // No catalogue ids: Bambu Studio's command-line program needs the list, not its values.
        "filament_ids": each(""),
        "filament_is_support": each("0"),
        "filament_settings_id": each(&presets.filament),
        "filament_shrink": each("100%"),
        "filament_type": each(kind),
        "from": "project",
        "name": "project_settings",
        "nozzle_diameter": [nozzle(&presets.printer)],
        "print_settings_id": presets.process,
        "printable_height": PRINTABLE_HEIGHT,
        "printer_settings_id": presets.printer,
        "version": BAMBU_VERSION,
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

fn package(parts: &[Part], slots: &[FilamentSlot], presets: &BambuPresets, metadata: &[(&str, &str)], paint: bool) -> Result<Vec<u8>> {
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n <Default Extension=\"config\" ContentType=\"text/xml\"/>\n</Types>\n";
    let rels = format!(
        "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel-1\" Type=\"{MODEL_REL}\"/>\n</Relationships>\n"
    );
    let mut model_rels = format!("{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n");
    for k in 0..parts.len() {
        let _ = writeln!(model_rels, " <Relationship Target=\"/3D/Objects/object_{}.model\" Id=\"rel-{}\" Type=\"{MODEL_REL}\"/>", k + 1, k + 1);
    }
    model_rels.push_str("</Relationships>\n");
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, data: &[u8]| -> Result<()> {
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .large_file(data.len() as u64 >= u32::MAX as u64);
        zip.start_file(name, opts).map_err(|e| IoError::Invalid(format!("3MF: {e}")))?;
        zip.write_all(data).map_err(|e| IoError::Invalid(format!("3MF: {e}")))
    };
    add("[Content_Types].xml", content_types.as_bytes())?;
    add("_rels/.rels", rels.as_bytes())?;
    add("3D/3dmodel.model", root_model(parts, metadata).as_bytes())?;
    add("3D/_rels/3dmodel.model.rels", model_rels.as_bytes())?;
    for (k, p) in parts.iter().enumerate() {
        add(&format!("3D/Objects/object_{}.model", k + 1), object_part(p, 2 * k + 1, k, paint).as_bytes())?;
    }
    add("Metadata/model_settings.config", model_settings(parts).as_bytes())?;
    add("Metadata/project_settings.config", project_settings(slots, presets).as_bytes())?;
    zip.finish().map(|c| c.into_inner()).map_err(|e| IoError::Invalid(format!("3MF: {e}")))
}

#[cfg(test)]
#[path = "bambu_tests.rs"]
mod tests;
