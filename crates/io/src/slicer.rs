//! Mesh exports made for a slicer: the colours of bodies and of faces with colours of their own
//! become the slicer's filaments. [`SlicerTarget::Bambu`] writes a Bambu Studio project 3MF
//! (one filament per colour, see [`crate::bambu`]) or an OBJ file with its colours in an MTL file
//! next to it; a plain OBJ export can carry the MTL file too ([`SlicerOptions::mtl`]).
//!
//! Colours are reduced to at most `max_colors` by merging the closest ones as the eye sees them:
//! agglomerative clustering in CIELAB (D65) weighted by the area each colour covers, merging the
//! pair whose union adds the least variance (Ward: ΔE² · w₁w₂ / (w₁ + w₂)) until the limit is
//! reached, each slot taking the area-weighted mean. Near-identical greys fuse with each other
//! before a small but distinct colour (a cyan LED, a red wire) is touched. Slots are numbered in
//! the order the bodies first use them. With no slicer target and no MTL file nothing here runs,
//! and exports are written exactly as before.

use std::collections::HashMap;

use solidcraft_doc::{Metadata, ModelState};

use crate::threemf::MeshObject;
use crate::{Format, IoError, Result};

/// The most filaments a Bambu Studio project made here carries (Bambu Studio takes 32).
pub const MAX_BAMBU_FILAMENTS: usize = 32;
/// The filaments a Bambu Studio project gets when no limit is given (one AMS chain of four units).
pub const DEFAULT_BAMBU_FILAMENTS: usize = 16;
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
    /// MTL file, [`DEFAULT_BAMBU_FILAMENTS`] for Bambu Studio (at most [`MAX_BAMBU_FILAMENTS`]).
    pub max_colors: Option<usize>,
    /// Faces with colours of their own keep them; false: each body is all its own colour.
    pub face_colors: bool,
    /// OBJ: write the colours as an MTL file next to it (always for Bambu Studio).
    pub mtl: bool,
    /// The bodies as one assembly: in a Bambu Studio project one object with a part per body
    /// (each with its filament), in an OBJ one object with a group per body. Separate objects
    /// are each set down on the plate by Bambu Studio, which breaks up an assembly.
    pub assembly: bool,
    pub presets: BambuPresets,
}

impl SlicerOptions {
    /// The options that change nothing: the plain export.
    pub fn plain(&self) -> bool {
        self.target == SlicerTarget::None && !self.mtl
    }

    /// The most colours the export may use.
    pub(crate) fn limit(&self) -> usize {
        let (default, cap) =
            if self.target == SlicerTarget::Bambu { (DEFAULT_BAMBU_FILAMENTS, MAX_BAMBU_FILAMENTS) } else { (MAX_MTL_MATERIALS, MAX_MTL_MATERIALS) };
        self.max_colors.unwrap_or(default).clamp(1, cap)
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

/// sRGB (8 bits) to CIELAB, D65 white.
pub(crate) fn lab(c: Rgba) -> [f64; 3] {
    let lin = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let f = |t: f64| if t > 216.0 / 24389.0 { t.cbrt() } else { t * 841.0 / 108.0 + 4.0 / 29.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIELAB (D65) back to 8-bit sRGB, clamped to the gamut.
pub(crate) fn srgb(l: [f64; 3]) -> [u8; 3] {
    let fy = (l[0] + 16.0) / 116.0;
    let (fx, fz) = (fy + l[1] / 500.0, fy - l[2] / 200.0);
    let inv = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 108.0 / 841.0 * (t - 4.0 / 29.0) };
    let (x, y, z) = (0.95047 * inv(fx), inv(fy), 1.08883 * inv(fz));
    let r = 3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let g = -0.9692660 * x + 1.8760108 * y + 0.0415560 * z;
    let b = 0.0556434 * x - 0.2040259 * y + 1.0572252 * z;
    let gamma = |c: f64| {
        let c = if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
        let v = if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [gamma(r), gamma(g), gamma(b)]
}

/// The use of one colour: the triangles and the area it covers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorUse {
    pub color: Rgba,
    pub triangles: usize,
    /// mm².
    pub area: f64,
}

/// One slot (filament or material) of a palette: its colour, what it covers and the colours of
/// the design it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub color: Rgba,
    pub triangles: usize,
    /// mm².
    pub area: f64,
    /// The design's colours that take this slot, in order of first use.
    pub sources: Vec<Rgba>,
}

/// The colours an export uses, reduced to a limit.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// The slots, in the order the bodies first use them (filament 1 first).
    pub slots: Vec<Slot>,
    /// Every colour used and its slot.
    slot: HashMap<Rgba, usize>,
}

/// The most colours clustered one by one; beyond it colours are first pooled per 4 bits a
/// channel (a design rarely has more than a few hundred).
const MAX_CLUSTER_INPUT: usize = 4096;

/// A cluster being built: Lab mean, mean alpha, weight, members (indices into `used`).
struct Cluster {
    lab: [f64; 3],
    alpha: f64,
    weight: f64,
    members: Vec<usize>,
}

/// The variance a merge adds (Ward).
fn ward(a: &Cluster, b: &Cluster) -> f64 {
    let d: f64 = (0..3).map(|i| (a.lab[i] - b.lab[i]).powi(2)).sum();
    d * a.weight * b.weight / (a.weight + b.weight)
}

impl Palette {
    /// `used`: each colour and what it covers, in order of first use. At most `max` slots (at
    /// least one): every colour its own while they fit, else merged as the module says.
    pub fn new(used: &[ColorUse], max: usize) -> Palette {
        let max = max.max(1);
        // Weights: the area, or the triangles when there is no area at all.
        let total: f64 = used.iter().map(|u| if u.area.is_finite() { u.area.max(0.0) } else { 0.0 }).sum();
        let weight = |u: &ColorUse| {
            let w = if total > 0.0 && u.area.is_finite() { u.area.max(0.0) } else { u.triangles as f64 };
            w.max(1e-12)
        };
        let mut clusters: Vec<Cluster> = Vec::new();
        if used.len() > MAX_CLUSTER_INPUT {
            let mut pool: HashMap<[u8; 4], usize> = HashMap::new();
            for (i, u) in used.iter().enumerate() {
                let key = u.color.map(|v| v >> 4);
                match pool.get(&key) {
                    Some(&k) => {
                        if let Some(c) = clusters.get_mut(k) {
                            let w = weight(u);
                            let l = lab(u.color);
                            let t = c.weight + w;
                            for (m, v) in c.lab.iter_mut().zip(l) {
                                *m = (*m * c.weight + v * w) / t;
                            }
                            c.alpha = (c.alpha * c.weight + f64::from(u.color[3]) * w) / t;
                            c.weight = t;
                            c.members.push(i);
                        }
                    }
                    None => {
                        pool.insert(key, clusters.len());
                        clusters.push(Cluster { lab: lab(u.color), alpha: f64::from(u.color[3]), weight: weight(u), members: vec![i] });
                    }
                }
            }
        } else {
            clusters = used
                .iter()
                .enumerate()
                .map(|(i, u)| Cluster { lab: lab(u.color), alpha: f64::from(u.color[3]), weight: weight(u), members: vec![i] })
                .collect();
        }
        merge_down(&mut clusters, max);
        // Slots in the order of first use (a cluster's first member).
        clusters.retain(|c| !c.members.is_empty());
        for c in &mut clusters {
            c.members.sort_unstable();
        }
        clusters.sort_by_key(|c| c.members.first().copied().unwrap_or(usize::MAX));
        let mut slot = HashMap::new();
        let mut slots = Vec::with_capacity(clusters.len());
        for (k, c) in clusters.iter().enumerate() {
            let members: Vec<&ColorUse> = c.members.iter().filter_map(|i| used.get(*i)).collect();
            let color = match members.as_slice() {
                [only] => only.color,
                _ => {
                    let [r, g, b] = srgb(c.lab);
                    [r, g, b, c.alpha.round().clamp(0.0, 255.0) as u8]
                }
            };
            for m in &members {
                slot.insert(m.color, k);
            }
            slots.push(Slot {
                color,
                triangles: members.iter().map(|m| m.triangles).sum(),
                area: members.iter().map(|m| m.area).sum(),
                sources: members.iter().map(|m| m.color).collect(),
            });
        }
        Palette { slots, slot }
    }

    /// The slot colours.
    pub fn colors(&self) -> Vec<Rgba> {
        self.slots.iter().map(|s| s.color).collect()
    }

    /// Colours that share their slot with another (those merged away by the limit).
    pub fn merged(&self) -> usize {
        self.slots.iter().map(|s| s.sources.len().saturating_sub(1)).sum()
    }

    /// The slot (0-based) of a colour; a colour never seen takes the nearest slot in Lab.
    pub fn slot(&self, c: Rgba) -> usize {
        self.slot.get(&c).copied().unwrap_or_else(|| {
            let l = lab(c);
            let d = |s: &Slot| {
                let m = lab(s.color);
                (0..3).map(|i| (m[i] - l[i]).powi(2)).sum::<f64>()
            };
            self.slots.iter().enumerate().min_by(|(i, a), (j, b)| d(a).total_cmp(&d(b)).then(i.cmp(j))).map_or(0, |(i, _)| i)
        })
    }
}

/// Merge clusters (Ward, the cheapest pair first; ties: the lowest indices) until `max` are left.
/// Each cluster keeps its nearest neighbour and that cost; a merge recomputes only the clusters
/// whose neighbour it touched.
fn merge_down(clusters: &mut [Cluster], max: usize) {
    let n = clusters.len();
    if n <= max {
        return;
    }
    let mut alive = vec![true; n];
    let mut nn = vec![(f64::INFINITY, usize::MAX); n];
    let nearest = |clusters: &[Cluster], alive: &[bool], i: usize| -> (f64, usize) {
        let mut best = (f64::INFINITY, usize::MAX);
        if let Some(a) = clusters.get(i) {
            for (j, b) in clusters.iter().enumerate() {
                if j != i && alive.get(j).copied().unwrap_or(false) {
                    let c = ward(a, b);
                    if c < best.0 || (c == best.0 && j < best.1) {
                        best = (c, j);
                    }
                }
            }
        }
        best
    };
    for (i, slot) in nn.iter_mut().enumerate() {
        *slot = nearest(clusters, &alive, i);
    }
    let mut left = n;
    while left > max {
        // The cheapest merge.
        let Some((i, (_, j))) = nn
            .iter()
            .enumerate()
            .filter(|(i, (_, j))| alive.get(*i).copied().unwrap_or(false) && *j != usize::MAX)
            .min_by(|(i, a), (k, b)| a.0.total_cmp(&b.0).then(i.cmp(k)))
            .map(|(i, v)| (i, *v))
        else {
            break;
        };
        let (keep, gone) = (i.min(j), i.max(j));
        let taken = clusters.get_mut(gone).map(|c| (c.lab, c.alpha, c.weight, std::mem::take(&mut c.members)));
        if let (Some((l, alpha, w, members)), Some(c)) = (taken, clusters.get_mut(keep)) {
            let t = c.weight + w;
            for (m, v) in c.lab.iter_mut().zip(l) {
                *m = (*m * c.weight + v * w) / t;
            }
            c.alpha = (c.alpha * c.weight + alpha * w) / t;
            c.weight = t;
            c.members.extend(members);
        }
        if let Some(a) = alive.get_mut(gone) {
            *a = false;
        }
        left -= 1;
        if let Some(v) = nn.get_mut(gone) {
            *v = (f64::INFINITY, usize::MAX);
        }
        let fresh = nearest(clusters, &alive, keep);
        if let Some(v) = nn.get_mut(keep) {
            *v = fresh;
        }
        for k in 0..n {
            if k == keep || !alive.get(k).copied().unwrap_or(false) {
                continue;
            }
            let Some(&(cost, near)) = nn.get(k) else { continue };
            if near == keep || near == gone {
                let v = nearest(clusters, &alive, k);
                if let Some(slot) = nn.get_mut(k) {
                    *slot = v;
                }
            } else if let (Some(a), Some(b)) = (clusters.get(k), clusters.get(keep)) {
                let c = ward(a, b);
                if (c < cost || (c == cost && keep < near))
                    && let Some(slot) = nn.get_mut(k)
                {
                    *slot = (c, keep);
                }
            }
        }
    }
}

/// Colours in order of first use and what each covers.
#[derive(Default)]
pub(crate) struct Usage {
    order: Vec<ColorUse>,
    index: HashMap<Rgba, usize>,
}

impl Usage {
    /// One triangle of colour `c` and area `area` (mm²).
    pub(crate) fn add(&mut self, c: Rgba, area: f64) {
        let area = if area.is_finite() { area.max(0.0) } else { 0.0 };
        match self.index.get(&c) {
            Some(&i) => {
                if let Some(u) = self.order.get_mut(i) {
                    u.triangles += 1;
                    u.area += area;
                }
            }
            None => {
                self.index.insert(c, self.order.len());
                self.order.push(ColorUse { color: c, triangles: 1, area });
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

/// The area of a triangle of `v`.
pub(crate) fn area(v: &[[f64; 3]], t: [u32; 3]) -> f64 {
    let (Some(a), Some(b), Some(c)) = (v.get(t[0] as usize), v.get(t[1] as usize), v.get(t[2] as usize)) else { return 0.0 };
    let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let x = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt()
}

/// What a slicer export wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct SlicerExport {
    /// The file itself (3MF or OBJ).
    pub bytes: Vec<u8>,
    /// The MTL file that goes next to an OBJ file.
    pub mtl: Option<Vec<u8>>,
    /// The colours as written: filaments (Bambu Studio) or materials (MTL), with what each
    /// covers and the design's colours it stands for.
    pub colors: Vec<Slot>,
    /// Distinct colours in the bodies before the limit.
    pub distinct_colors: usize,
    /// Colours that share their filament or material with another.
    pub merged_colors: usize,
    /// Triangles painted with a filament other than their body's (Bambu Studio 3MF).
    pub painted_triangles: usize,
    /// The filament (1-based) of each body, in order (Bambu Studio 3MF).
    pub extruders: Vec<usize>,
    /// Bodies written.
    pub bodies: usize,
}

/// Export `bodies` (all when empty) of `state` for a slicer: a Bambu Studio project 3MF, or OBJ
/// with an MTL file named `mtl_name` (the file name the OBJ refers to). `name` names an assembly. Plain options are refused:
/// those exports go through [`crate::export_with_metadata`].
pub fn export_for_slicer(
    state: &ModelState,
    bodies: &[String],
    format: Format,
    name: &str,
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
        (SlicerTarget::Bambu, Format::ThreeMf) => crate::bambu::write_bambu_3mf(&objects()?, options, name, &crate::bambu_metadata(meta)),
        (_, Format::Obj) if options.target == SlicerTarget::Bambu || options.mtl => {
            Ok(crate::obj_out::obj_with_mtl(&objects()?, options, name, mtl_name))
        }
        (SlicerTarget::Bambu, _) => Err(IoError::Invalid("Bambu Studio export: the format must be 3mf or obj".into())),
        _ => Err(IoError::Invalid("an MTL file goes with an OBJ export only".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = [255, 0, 0, 255];
    const BLUE: Rgba = [0, 64, 255, 255];
    const CYAN: Rgba = [0, 200, 220, 255];
    const GREY1: Rgba = [128, 128, 128, 255];
    const GREY2: Rgba = [132, 131, 130, 255];
    const GREY3: Rgba = [125, 127, 129, 255];

    fn used(c: Rgba, area: f64) -> ColorUse {
        ColorUse { color: c, triangles: 2, area }
    }

    /// Three near greys covering most of the area and one small cyan: down to two slots the
    /// greys fuse into one (their mean) and the cyan keeps its own, though it covers the least.
    #[test]
    fn near_greys_fuse_and_a_rare_distinct_colour_stays() {
        let colours = [used(GREY1, 5000.0), used(CYAN, 10.0), used(GREY2, 4000.0), used(GREY3, 3000.0)];
        let p = Palette::new(&colours, 2);
        assert_eq!(p.slots.len(), 2);
        assert_eq!(p.slots[1].color, CYAN, "the cyan keeps its own slot: {:?}", p.slots);
        assert_eq!(p.slots[1].sources, [CYAN]);
        assert_eq!(p.slots[0].sources, [GREY1, GREY2, GREY3], "first use order");
        let g = p.slots[0].color;
        assert!(g.iter().take(3).all(|v| (125..=132).contains(v)), "a grey between them: {g:?}");
        assert_eq!((p.slots[0].triangles, p.slots[0].area), (6, 12000.0));
        assert_eq!((p.slot(GREY3), p.slot(CYAN)), (0, 1));
        assert_eq!(p.merged(), 2);
        // The old rule (keep the most used) would have dropped the cyan: it covers the least.
        assert!(colours.iter().all(|u| u.area >= 10.0));
    }

    /// While the colours fit, each keeps its own exact colour, in order of first use; a limit of 0
    /// still keeps one slot; nothing is no slot.
    #[test]
    fn colours_that_fit_are_kept_exactly() {
        let colours = [used(BLUE, 1.0), used(RED, 50.0), used(CYAN, 2.0)];
        let p = Palette::new(&colours, 16);
        assert_eq!(p.colors(), [BLUE, RED, CYAN]);
        assert_eq!(p.merged(), 0);
        let one = Palette::new(&colours, 0);
        assert_eq!(one.slots.len(), 1);
        assert_eq!(one.slots[0].sources.len(), 3);
        assert!(Palette::new(&[], 4).slots.is_empty());
        // A colour never seen goes to the nearest slot.
        assert_eq!(p.slot([250, 10, 10, 255]), 1);
        // No area at all: the triangles weigh instead.
        let flat = [
            ColorUse { color: GREY1, triangles: 5, area: 0.0 },
            ColorUse { color: GREY2, triangles: 5, area: 0.0 },
            ColorUse { color: CYAN, triangles: 1, area: 0.0 },
        ];
        assert_eq!(Palette::new(&flat, 2).slots[1].color, CYAN);
    }

    /// Many colours (over the clustering cap) still come down to the limit, quickly and without
    /// panicking on hostile weights.
    #[test]
    fn many_colours_come_down_to_the_limit() {
        let colours: Vec<ColorUse> = (0..6000u32)
            .map(|i| ColorUse {
                color: [(i % 256) as u8, ((i / 256) * 9 % 256) as u8, (i * 7 % 256) as u8, 255],
                triangles: 1,
                area: if i % 3 == 0 { f64::NAN } else { i as f64 },
            })
            .collect();
        let t = std::time::Instant::now();
        let p = Palette::new(&colours, 32);
        assert_eq!(p.slots.len(), 32);
        assert_eq!(p.slots.iter().map(|s| s.sources.len()).sum::<usize>(), colours.len());
        assert!(t.elapsed() < std::time::Duration::from_secs(60));
    }

    #[test]
    fn lab_round_trips_and_colours_stay_in_range() {
        for c in [[0u8, 0, 0], [255, 255, 255], [255, 0, 0], [0, 64, 255], [18, 200, 33], [179, 179, 179]] {
            assert_eq!(srgb(lab([c[0], c[1], c[2], 255])), c);
        }
        assert_eq!(srgb([f64::NAN, 0.0, 0.0]), [0, 0, 0]);
        assert_eq!(srgb([200.0, 300.0, -300.0]).len(), 3);
        assert_eq!(rgba([1.0, 0.5, 0.0], 0.0), [255, 128, 0, 255]);
        assert_eq!(rgba([f32::NAN, 2.0, -1.0], 0.5), [0, 255, 0, 128]);
        assert_eq!(rgba([0.7, 0.7, 0.7], f32::INFINITY), [179, 179, 179, 255]);
        assert_eq!(hex([0, 64, 255, 10]), "#0040FF");
        assert_eq!(SlicerTarget::parse(" Bambu "), Some(SlicerTarget::Bambu));
        assert_eq!(SlicerTarget::parse("none"), Some(SlicerTarget::None));
        assert_eq!(SlicerTarget::parse("prusa"), None);
        assert!((area(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 0.0]], [0, 1, 2]) - 3.0).abs() < 1e-12);
        assert_eq!(area(&[], [0, 1, 2]), 0.0);
    }
}
