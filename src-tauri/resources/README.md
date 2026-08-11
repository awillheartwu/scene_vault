# Bundled resources

This directory is packaged into the installer through `bundle.resources` in
`tauri.conf.json`. `resources/fonts` ships in every installer; `resources/models`
and `binaries/` are populated by `scripts/release-windows.ps1` and only shipped
in the AI-enabled installer (see `tauri.ai.conf.json`).

## fonts

- `SmileySans-Oblique.ttf`（得意黑）— SIL Open Font License 1.1, redistributable.
  Official release: <https://github.com/atelier-anchor/smiley-sans/releases>
  (v2.0.1). The full license text (`OFL.txt`) is included in the official zip;
  the release script refreshes both the font and the license before building.

## models (populated by the release script, never committed)

- `face_detection_yunet_2023mar.onnx`（YuNet, ~227 KB）
- `face_recognition_sface_2021dec.onnx`（SFace, ~37 MB）

ArcFace weights (`w600k_r50.onnx`) stay user-provided and are never bundled.
Sources: opencv_zoo on GitHub (see `scripts/setup-windows.ps1`).
