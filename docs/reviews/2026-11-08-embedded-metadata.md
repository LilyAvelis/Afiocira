# Embedded metadata and copy-path actions

Ctrl+G now separates editable Afio notes from read-only information embedded in
the requested image. A collapsible notes section, searchable format groups,
namespace-qualified field identities and complete raw-payload copy establish a
base for future field visibility/reordering. Visibility settings and editing
embedded fields are not part of this change.

## Readers

- XMP: standard JPEG APP1, PNG iTXt and WebP XMP; generic namespace-expanded RDF
  properties and arrays, original XML. Extended JPEG XMP is reported as
  incomplete/unsupported rather than silently reconstructed.
- EXIF: supported TIFF payloads in JPEG/PNG/WebP and small TIFF files through
  kamadak-exif, with a preflight bound on IFD graph/entries. Camera, orientation,
  exposure, timestamps, lens, GPS and other available tags remain descriptive
  data; no library scan is involved.
- IPTC-IIM: supported Photoshop JPEG resources, including author, caption,
  keywords and copyright. Unsupported encodings/structures are bounded readers,
  not a claim of complete IPTC support.
- ICC and image headers: format, dimensions, PNG color/bit depth, available ICC
  identity/description. Compressed PNG ICC currently reports the profile name;
  this is inspection, not color-managed editing.
- Generation: PNG tEXt/zTXt/iTXt, ComfyUI prompt/workflow JSON, generic prompt
  JSON and A1111 parameters. Full raw JSON/text is retained within its budget
  and can be copied; ordinary PNG text is a separate group.
- Potential provenance/JUMBF containers are reported without verifying
  signatures. Generic JUMBF alone does not set the C2PA indicator.

Reader work runs on the existing explicit metadata worker. Opening Ctrl+G/API
Inspect reads only its target and creates no Registry record. File reads cap at
64 MiB, raw metadata at 1 MiB, displayed properties at 256, values at 4096
characters, warnings at 32. XML rejects DTD/entities and limits nodes/depth;
compressed text is bounded after decompression. The whole raw workflow/XML is
kept or omitted with a warning; raw JSON is never cut into an invalid document.
Windows read locks and before/after source-version checks protect inspection.

## Saving images

Photometric JPEG edits now preserve supported standard XMP APP1 byte-for-byte.
The user's Gemini example contains ordinary Google AI credit/digital-source/date
XMP; a copied example passed preview/apply with unchanged XMP and the original
file's hash unchanged. Original files were inspected only, not edited.

PNG international text, including compressed ComfyUI workflow, survives
photometric save. Arbitrary metadata rewriting is not enabled: geometric
rotation/reflection with JPEG XMP, EXIF auto-orientation combined with XMP,
extended XMP, signed provenance and unsupported APP blocks retain explicit
refusals instead of silently losing data or claiming a valid signature.
WebP XMP is readable but not yet preserved through editing. Existing sRGB,
precision and memory constraints remain.

## Path commands

`copy_path` defaults to Ctrl+L: current folder in Gallery, current file in Space.
`copy_selected_path` defaults to Ctrl+Shift+L in Gallery: cursor image path,
independent of marked files. Both share Actions/shortcut customization and text
clipboard output, with normal absolute Windows/UNC paths. Terminal/Yazi keeps
its local Ctrl+L. User bindings are not rewritten.

## Evidence

- 287 workspace tests plus 3 doctests passed. Focused metadata UI tests passed
  again after the collapsible notes refinement; strict Clippy/fmt/diff passed.
- Synthetic readers cover RDF arrays/namespaces, malformed/oversized XML,
  decompression limits, corrupt containers, EXIF graph amplification, IPTC, ICC,
  Comfy JSON and recovery. Geometry tests cover narrow/normal/wide and DPI1/1.5/2.
- Native fixture `target/embedded-bae49d2f/results.json`: read-only sparse
  inspection, full workflow retention across PNG edit, JPEG XMP preservation,
  and editing only a copy of the supplied example; original hash unchanged.
- Native existing regression: `target/edit-panels-265be481` and sparse
  `target/metadata-023c4a9cbe` (101 images, one explicitly registered).
- Added pinned kamadak-exif 0.6.1; XML/compression use already locked roxmltree
  0.20.0/flate2 1.1.11. Generated Rust dependency notices updated.
- Final native fixture after UI refinement: `target/embedded-73214478` passed
  all checks, including the copied Gemini example. Working version
  `80d6f76bab25` installed through the permanent desktop shortcut; executable
  hash matches the tested build. Existing user windows were not closed.

## Primary references

- [Adobe XMP](https://developer.adobe.com/xmp/docs/)
- [ComfyUI SaveImage](https://github.com/Comfy-Org/ComfyUI/blob/master/nodes.py)
- [A1111 image metadata](https://github.com/AUTOMATIC1111/stable-diffusion-webui/blob/master/modules/images.py)
- [IPTC IIM](https://www.iptc.org/std/IIM/4.2/specification/IIMV4.2.pdf)
- [ICC specifications](https://www.color.org/icc_specs2/)
- [PNG eXIf](https://www.w3.org/TR/png-3/#11eXIf)
- [WebP RIFF metadata](https://developers.google.com/speed/webp/docs/riff_container)
- [EXIF parser](https://github.com/kamadak/exif-rs)
