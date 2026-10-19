//! Mesh exports made for a slicer: the colours of bodies and of faces with colours of their own
//! become the slicer's filaments. [`SlicerTarget::Bambu`] writes a Bambu Studio project 3MF
//! (one filament per colour, see [`crate::bambu`]) or an OBJ file with its colours in an MTL file
//! next to it; a plain OBJ export can carry the MTL file too ([`SlicerOptions::mtl`]).
//!
//! Colours are reduced to at most `max_colors`: the colours that cover the most triangles keep
//! a filament of their own (numbered in the order the bodies first use them), and every other
//! colour takes the nearest kept one in RGB. With no slicer target and no MTL file nothing here
//! runs, and exports are written exactly as before.

use std::collections::HashMap;

use solidcraft_doc::{Metadata, ModelState};

use crate::threemf::MeshObject;
use crate::{Format, IoError, Result};

/// The most filaments a Bambu Studio project made here carries.
pub const MAX_BAMBU_FILAMENTS: usize = 16;
/// The most materials an OBJ file's MTL file gets when no limit is given.
pub const MAX_MTL_MATERIALS: usize = 4096;
/// The colour of a body with no colour of its own (as the 3MF export writes it).
pub(crate) const DEFAULT_COLOR: [f32; 3] = [0.7, 0.7, 0.7];

/// The slicer a mesh export is made for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SlicerTarget {
    /// Any program: the export is the plain format.
    #[default]
    None,
    /// Bambu Studio: a project 3MF with one filament per colour, or OBJ with an MTL file.
    Bambu,
}

impl SlicerTarget {
    /// `none` or `bambu` (any case); `None` for anything else.
    pub fn parse(s: &str) -> Option<SlicerTarget> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" => Some(SlicerTarget::None),
            "bambu" | "bambu-studio" | "bambu_studio" => Some(SlicerTarget::Bambu),
            _ => None,
        }
    }
}

/// The Bambu Studio presets a project names: printer, process and the filament every slot uses
/// (its colour is the slot's own). They are names only: Bambu Studio fills in their settings.
#[derive(Clone, Debug, PartialEq)]
pub struct BambuPresets {
    pub printer: String,
    pub process: String,
    pub filament: String,
}

impl Default for BambuPresets {
    fn default() -> Self {
        BambuPresets {
            printer: "Bambu Lab P2S 0.4 nozzle".into(),
            process: "0.20mm Standard @BBL P2S".into(),
            filament: "Bambu PLA Basic @BBL P2S".into(),
        }
    }
}

/// How a mesh export is made for a slicer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlicerOptions {
    pub target: SlicerTarget,
    /// The most colours (filaments, or MTL materials) in the file. `None`: every colour for an
    /// MTL file, [`MAX_BAMBU_FILAMENTS`] for Bambu Studio.
    pub max_colors: Option<usize>,
    /// Faces with colours of their own keep them; false: each body is all its own colour.
    pub face_colors: bool,
    /// OBJ: write the colours as an MTL file next to it (always for Bambu Studio).
    pub mtl: bool,
    pub presets: BambuPresets,
}

impl SlicerOptions {
    /// The options that change nothing: the plain export.
    pub fn plain(&self) -> bool {
        self.target == SlicerTarget::None && !self.mtl
    }

    /// The most colours the export may use.
    pub(crate) fn limit(&self) -> usize {
        let cap = if self.target == SlicerTarget::Bambu { MAX_BAMBU_FILAMENTS } else { MAX_MTL_MATERIALS };
        self.max_colors.unwrap_or(cap).clamp(1, cap)
    }
}

/// A colour as written: sRGB and alpha, 8 bits each.
pub type Rgba = [u8; 4];

/// A 0..1 colour and its transparency as 8-bit RGBA (alpha 255 = opaque).
pub(crate) fn rgba(c: [f32; 3], transparency: f32) -> Rgba {
    let byte = |v: f32| if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
    let t = if transparency.is_finite() { transparency.clamp(0.0, 1.0) } else { 0.0 };
    [byte(c[0]), byte(c[1]), byte(c[2]), byte(1.0 - t)]
}

/// `#RRGGBB`.
pub(crate) fn hex(c: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// The colours an export uses, reduced to a limit.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// The kept colours, in the order the bodies first use them (filament 1 first).
    pub colors: Vec<Rgba>,
    /// Triangles each kept colour covers once merged colours are added.
    pub triangles: Vec<usize>,
    /// Every colour used and its slot in `colors`.
    slot: HashMap<Rgba, usize>,
    /// Colours that took the slot of another (over the limit).
    pub merged: usize,
}

impl Palette {
    /// `used`: each colour and the triangles it covers, in order of first use. The `max` colours
    /// covering the most triangles are kept (ties: the first used); the rest take the nearest
    /// kept colour in RGB (ties: the first kept).
    pub fn new(used: &[(Rgba, usize)], max: usize) -> Palette {
        let mut order: Vec<(usize, &(Rgba, usize))> = used.iter().enumerate().collect();
        order.sort_by(|(i, (_, a)), (j, (_, b))| b.cmp(a).then(i.cmp(j)));
        let mut kept: Vec<usize> = order.iter().take(max.max(1)).map(|(i, _)| *i).collect();
        kept.sort_unstable();
        let colors: Vec<Rgba> = kept.iter().filter_map(|i| used.get(*i).map(|u| u.0)).collect();
        let mut slot = HashMap::new();
        let mut triangles = vec![0usize; colors.len()];
        let mut merged = 0;
        for (c, n) in used {
            let s = match colors.iter().position(|k| k == c) {
                Some(s) => s,
                None => {
                    merged += 1;
                    nearest(*c, &colors)
                }
            };
            slot.insert(*c, s);
            if let Some(t) = triangles.get_mut(s) {
                *t += n;
            }
        }
        Palette { colors, triangles, slot, merged }
    }

    /// The slot (0-based) of a colour; a colour never seen takes the nearest one.
    pub fn slot(&self, c: Rgba) -> usize {
        self.slot.get(&c).copied().unwrap_or_else(|| nearest(c, &self.colors))
    }
}

/// The index of the colour of `pool` nearest to `c` in RGB (the first of equals; 0 for none).
fn nearest(c: Rgba, pool: &[Rgba]) -> usize {
    let d = |p: &Rgba| (0..3).map(|i| (i32::from(c[i]) - i32::from(p[i])).pow(2)).sum::<i32>();
    pool.iter().enumerate().min_by_key(|(i, p)| (d(p), *i)).map_or(0, |(i, _)| i)
}

/// Colours in order of first use and the triangles each covers.
#[derive(Default)]
pub(crate) struct Usage {
    order: Vec<(Rgba, usize)>,
    index: HashMap<Rgba, usize>,
}

impl Usage {
    pub(crate) fn add(&mut self, c: Rgba, n: usize) {
        match self.index.get(&c) {
            Some(&i) => {
                if let Some(u) = self.order.get_mut(i) {
                    u.1 += n;
                }
            }
            None => {
                self.index.insert(c, self.order.len());
                self.order.push((c, n));
            }
        }
    }

    pub(crate) fn palette(&self, max: usize) -> Palette {
        Palette::new(&self.order, max)
    }

    pub(crate) fn distinct(&self) -> usize {
        self.order.len()
    }
}

/// What a slicer export wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct SlicerExport {
    /// The file itself (3MF or OBJ).
    pub bytes: Vec<u8>,
    /// The MTL file that goes next to an OBJ file.
    pub mtl: Option<Vec<u8>>,
    /// The colours as written: filaments (Bambu Studio) or materials (MTL), and the triangles of each.
    pub colors: Vec<(String, usize)>,
    /// Distinct colours in the bodies before the limit.
    pub distinct_colors: usize,
    /// Colours that took the filament or material of the nearest kept colour.
    pub merged_colors: usize,
    /// Triangles painted with a filament other than their body's (Bambu Studio 3MF).
    pub painted_triangles: usize,
    /// The filament (1-based) of each body, in order (Bambu Studio 3MF).
    pub extruders: Vec<usize>,
    /// Bodies written.
    pub bodies: usize,
}

/// Export `bodies` (all when empty) of `state` for a slicer: a Bambu Studio project 3MF, or OBJ
/// with an MTL file named `mtl_name` (the file name the OBJ refers to). Plain options are refused:
/// those exports go through [`crate::export_with_metadata`].
pub fn export_for_slicer(
    state: &ModelState,
    bodies: &[String],
    format: Format,
    meta: &Metadata,
    options: &SlicerOptions,
    mtl_name: &str,
) -> Result<SlicerExport> {
    let sel: Vec<&solidcraft_doc::ModelBody> = state.bodies.iter().filter(|b| bodies.is_empty() || bodies.contains(&b.name)).collect();
    if sel.is_empty() {
        return Err(IoError::Empty);
    }
    let objects = || sel.iter().map(|b| crate::mesh_object(b, false)).collect::<Result<Vec<MeshObject>>>();
    match (options.target, format) {
        (SlicerTarget::Bambu, Format::ThreeMf) => crate::bambu::write_bambu_3mf(&objects()?, options, &crate::bambu_metadata(meta)),
        (_, Format::Obj) if options.target == SlicerTarget::Bambu || options.mtl => Ok(crate::obj_out::obj_with_mtl(&objects()?, options, mtl_name)),
        (SlicerTarget::Bambu, _) => Err(IoError::Invalid("Bambu Studio export: the format must be 3mf or obj".into())),
        _ => Err(IoError::Invalid("an MTL file goes with an OBJ export only".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = [255, 0, 0, 255];
    const DARK_RED: Rgba = [200, 0, 0, 255];
    const BLUE: Rgba = [0, 64, 255, 255];
    const GREEN: Rgba = [0, 160, 0, 255];
    const YELLOW: Rgba = [255, 255, 0, 255];

    /// The most used colours keep their filaments, numbered in order of first use; the others
    /// take the nearest.
    #[test]
    fn palette_keeps_the_most_used_and_merges_the_rest_to_the_nearest() {
        let used = [(GREEN, 10), (RED, 24), (YELLOW, 2), (BLUE, 12), (DARK_RED, 1)];
        let all = Palette::new(&used, 16);
        assert_eq!(all.colors, [GREEN, RED, YELLOW, BLUE, DARK_RED]);
        assert_eq!(all.merged, 0);
        assert_eq!(all.slot(DARK_RED), 4);
        let three = Palette::new(&used, 3);
        assert_eq!(three.colors, [GREEN, RED, BLUE], "first-use order among the three most used");
        assert_eq!(three.merged, 2);
        assert_eq!(three.slot(DARK_RED), 1, "dark red joins red");
        assert_eq!(three.slot(YELLOW), 1, "yellow (255,255,0) is nearer red than green (0,160,0)");
        assert_eq!(three.triangles, [10, 27, 12]);
        // Ties keep the first used; a limit of 0 still keeps one colour.
        let tie = Palette::new(&[(BLUE, 5), (RED, 5)], 1);
        assert_eq!(tie.colors, [BLUE]);
        assert_eq!(Palette::new(&used, 0).colors.len(), 1);
        assert!(Palette::new(&[], 4).colors.is_empty());
    }

    #[test]
    fn colours_are_eight_bit_and_hostile_values_stay_in_range() {
        assert_eq!(rgba([1.0, 0.5, 0.0], 0.0), [255, 128, 0, 255]);
        assert_eq!(rgba([f32::NAN, 2.0, -1.0], 0.5), [0, 255, 0, 128]);
        assert_eq!(rgba([0.7, 0.7, 0.7], f32::INFINITY), [179, 179, 179, 255]);
        assert_eq!(hex([0, 64, 255, 10]), "#0040FF");
        assert_eq!(SlicerTarget::parse(" Bambu "), Some(SlicerTarget::Bambu));
        assert_eq!(SlicerTarget::parse("none"), Some(SlicerTarget::None));
        assert_eq!(SlicerTarget::parse("prusa"), None);
    }
}
