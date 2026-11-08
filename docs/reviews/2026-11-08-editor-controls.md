# Editor controls and falling curve — 2026-11-08

## User-visible correction

The curve validator incorrectly required nondecreasing output Y. The three UI
sliders are independent, so values 0.56, 0.63, 0.61 produced an error and disabled
Apply. The validator now accepts falling output segments while retaining finite
coordinates in [0,1], strictly increasing X, fixed endpoints and the 2–32 point
bound. This exact screenshot curve has core and native save regressions.

## Controls

The existing side panel now has fifteen manual adjustments plus the tone curve:

- Light: exposure, brilliance, contrast, shadows, highlights, brightness, black point.
- Color: saturation, vibrance, temperature, tint.
- Details: sharpness, definition, noise reduction.
- Effects: vignette.

Nine controls are new: brilliance, brightness, black point, saturation, vibrance,
sharpness, definition, noise reduction and vignette. Shadows/highlights were
already present. Groups collapse independently; Apply/Cancel/Reset stay outside
the scrolling controls, and the image remains on the central canvas.

The iPhone adjustment catalogue was a naming/scope reference, not an engine:
[current iPhone guide](https://support.apple.com/en-ca/guide/iphone/iphb08064d57/ios)
and [Apple Photos technical brief, 2019, pp. 11–13](https://www.apple.com/id/privacy/docs/Photos_Tech_Brief_Sept_2019.pdf).
Our deterministic sRGB processing does not reproduce Apple's proprietary
algorithms. Auto adjustment, crop/straighten and preset filters are separate
future work; this change does not claim full Photos parity.

## Processing and review

Preview and save share channel LUT, RGB color mixing, then spatial processing.
Cross-channel saturation/vibrance are not approximated with a one-dimensional
LUT. Spatial blur uses alpha-weighted separable bounded windows and two RGBA8
temporary buffers; hidden transparent RGB cannot tint visible neighbors, and
source alpha is retained. Existing conservative 256 MiB admission checks remain.

Prepared previews never upscale small files. Large files shrink to 1280 pixels;
spatial radii scale by the minimum dimension ratio to handle 1×5000-style images
correctly. Reduced previews remain approximate for fine details and nonlinear
processing; full-resolution save uses scale one. CPU work stays on workers.

Same-frame slider edits take priority over Apply so the saved options cannot
lag behind the controls. Preview writes neither source nor registry/history;
Apply creates one existing history transaction. Sparse indexing is unchanged.

## Evidence

- Workspace suite: 246 tests plus 3 doctests passed; subsequent narrow/tall-scale
  regression passed separately (247 distinct tests total). Strict workspace
  Clippy and formatting passed.
- Nine individual controls: lossless small-image saved pixels exactly match
  the preview; preview leaves source bytes intact.
- Native release fixture: `target/edit-panels-644db293/results.json`. Falling
  screenshot curve saves; combined new controls preview/apply and exact-byte
  undo pass. Existing conversion, metadata, thumbnails, curve and cancel checks
  pass. Root captures cover 680/1100/1600 logical widths; inspected narrow and
  normal panel captures. Existing shared geometry tests cover DPI 1/1.5/2.
- Sparse fixture: `target/metadata-37b64a07be`, 101 images, one explicitly
  registered file. Persistence, source integrity and optimistic revision checks
  pass.
- Lightweight Sol agents implemented color and spatial controls; an independent
  read-only review identified the tall-image scale edge case, fixed before
  publication.
- Static-CRT release installed as working version `2ed800aacd65`. Permanent
  desktop shortcut still points to AfioLauncher with the user's test-images
  argument. Installed executable SHA-256 matches the tested release. Existing
  user application windows were left running.

Physical keyboard and subjective acceptance on real photographs remain manual
checks; the native API evidence uses isolated synthetic fixtures.
