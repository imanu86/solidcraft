# Mesh exports for a slicer

`file.export` and `file.save_mesh` (and **File → Save as mesh…**, under **Slicer compatibility**)
can make a mesh file for a slicer, so that the colours of the design arrive as the slicer's
filaments. Without these options nothing changes: the file is the plain export, byte for byte.

| Parameter | Values | Default |
|---|---|---|
| `slicer_compat` | `none`, `bambu` | `none` |
| `mtl` | OBJ only: write the colours as an MTL file next to it | `false` (always on with `bambu`) |
| `max_colors` | most filaments or materials: 1 to 32 with `bambu` | 16 with `bambu`, else every colour |
| `assembly` | the bodies as one object, each where it is (a part per body in a 3MF, a group per body in an OBJ) | `false` |
| `face_colors` | faces with colours of their own keep them | `true` |
| `printer`, `process`, `filament` | Bambu Studio preset names (3MF only) | `Bambu Lab P2S 0.4 nozzle`, `0.20mm Standard @BBL P2S`, `Bambu PLA Basic @BBL P2S` |
| `visible_only` | any body export: only the bodies shown in the view | `false` |

## Colours

A body's colour is the one it is shown in (appearance, material or imported); a body with none is
grey (`#B3B3B3`), as in the 3MF export. A face with an appearance of its own has its own colour
when `face_colors` is on. While the colours fit in `max_colors` each keeps its own; over it they
are merged as the eye sees them: agglomerative clustering in CIELAB (D65) weighted by the area
each colour covers, always merging the pair that adds the least variance (Ward:
ΔE² · w₁w₂ / (w₁ + w₂)), each merged slot taking the area-weighted mean colour. Near-identical
greys fuse with each other before a small but distinct colour (a cyan LED, a red wire) is
touched. Slots are numbered in the order the bodies first use them. The command reports the
palette (`filaments` or `materials`: `color`, `triangles`, `area_mm2`, `merged` and the design's
colours in `sources`), `distinct_colors` and `merged_colors`.

## Bambu Studio project (`slicer_compat: "bambu"`, format `3mf`)

A 3MF package laid out the way Bambu Studio writes its own projects, so that it opens as a
project (with its filaments) rather than as geometry only:

- `3D/3dmodel.model`: core, production (`p`, required) and Bambu Studio namespaces; the metadata
  `Application` = `BambuStudio-02.08.02.61` (Bambu Studio reads other applications' 3MF files as
  geometry only) and `BambuStudio:3mfVersion` = `1`; per body an object holding one component
  in `3D/Objects/object_<n>.model`, placed by a build item. The model is centred on a 256 mm
  plate and stands on it; the bodies keep their places relative to each other.
- `Metadata/model_settings.config`: per object its name and `extruder`, the filament (from 1) that
  most of the body's triangles use.
- `Metadata/project_settings.config`: the filaments (`filament_colour`, `filament_type`,
  `filament_settings_id` and the other per-filament lists, one entry per filament) and the
  printer, process and filament presets by name. Settings left out take Bambu Studio's own.
- With `assembly`, one object instead: its components are the bodies, all in
  `3D/Objects/object_1.model`, each placed by the translation of its centre, and in the settings a
  `<part>` per body with its name, `matrix` (that placement) and `extruder`. Bambu Studio sets
  separate objects down on the plate each on its own, which breaks up an assembly; the parts of
  one object keep their places (checked: positions unchanged after loading, saving and slicing).
- Triangles of another colour than their body's filament carry `paint_color`: `4` for filament
  1, `8` for 2, `0C` … `EC` for 3 to 17 (the hexadecimal digit of the filament minus 3, then
  `C`) and `0FC` … `EFC` for 18 to 32 (the digit of the filament minus 18, then `FC`). Projects
  of up to 32 filaments load, slice and save again.

The layout was worked out by experiment with Bambu Studio's command-line program (2.8.2.61):
loading packages, exporting them again and slicing them, and looking at the files it wrote. No
Bambu Studio source was read. The smallest `project_settings.config` it loads, slices and saves
again holds the preset names, the nozzle diameter, the printable height and the per-filament
lists written here (`filament_ids` among them, even empty). The paint codes from 17 on were found
by slicing a face painted with each candidate code and weighing the filaments. Mixed filaments
(Bambu Studio 2.5.3 and later) are not written yet.

## OBJ with MTL

With `mtl: true` (or `slicer_compat: "bambu"` and format `obj`) the OBJ names an MTL file of
the same name next to it (`mtllib`) and switches material (`usemtl`) wherever the colour
changes (with `assembly`: one `o` named after the file and a `g` per body); the MTL file has one
material per colour (`solidcraft_RRGGBB`, `Kd`, and `d` for a
see-through colour, except for Bambu Studio, whose filaments are opaque). The bodies are welded
watertight as in the 3MF export, with single-precision coordinates and no normals. The MTL file is
written first, then the OBJ.
