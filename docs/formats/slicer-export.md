# Mesh exports for a slicer

`file.export` and `file.save_mesh` (and **File → Save as mesh…**, under **Slicer compatibility**)
can make a mesh file for a slicer, so that the colours of the design arrive as the slicer's
filaments. Without these options nothing changes: the file is the plain export, byte for byte.

| Parameter | Values | Default |
|---|---|---|
| `slicer_compat` | `none`, `bambu` | `none` |
| `mtl` | OBJ only: write the colours as an MTL file next to it | `false` (always on with `bambu`) |
| `max_colors` | most filaments or materials: 1 to 16 with `bambu` | 16 with `bambu`, else every colour |
| `face_colors` | faces with colours of their own keep them | `true` |
| `printer`, `process`, `filament` | Bambu Studio preset names (3MF only) | `Bambu Lab P2S 0.4 nozzle`, `0.20mm Standard @BBL P2S`, `Bambu PLA Basic @BBL P2S` |
| `visible_only` | any body export: only the bodies shown in the view | `false` |

## Colours

A body's colour is the one it is shown in (appearance, material or imported); a body with none is
grey (`#B3B3B3`), as in the 3MF export. A face with an appearance of its own has its own colour
when `face_colors` is on. Colours are counted by the triangles they cover. The `max_colors`
colours covering the most keep a filament (or material) of their own, numbered in the order the
bodies first use them; every other colour takes the nearest kept colour in RGB. The command
reports the colours written (`filaments` or `materials`, with their triangles), `distinct_colors`
and `merged_colors`.

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
- Triangles of another colour than their body's filament carry `paint_color`: `4` for filament
  1, `8` for 2, and from 3 on the hexadecimal digit of the filament number minus 3 followed by `C`
  (`0C` … `DC` for 16).

The layout was worked out by experiment with Bambu Studio's command-line program (2.8.2.61):
loading packages, exporting them again and slicing them, and looking at the files it wrote. No
Bambu Studio source was read. The smallest `project_settings.config` it loads, slices and saves
again holds the preset names, the nozzle diameter, the printable height and the per-filament
lists written here (`filament_ids` among them, even empty). Mixed filaments (Bambu Studio 2.5.3
and later) are not written yet.

## OBJ with MTL

With `mtl: true` (or `slicer_compat: "bambu"` and format `obj`) the OBJ names an MTL file of
the same name next to it (`mtllib`) and switches material (`usemtl`) wherever the colour
changes; the MTL file has one material per colour (`solidcraft_RRGGBB`, `Kd`, and `d` for a
see-through colour, except for Bambu Studio, whose filaments are opaque). The bodies are welded
watertight as in the 3MF export, with single-precision coordinates and no normals. The MTL file is
written first, then the OBJ.
