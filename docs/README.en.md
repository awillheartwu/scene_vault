# Scene Vault

[简体中文](../README.md) | English

Scene Vault is a **local-first Windows desktop app** for screenshot management and
creative workflow support across games, built with Tauri 2, Rust, Vue 3 and
SQLite. An optional local Python vision engine provides face detection,
character-name annotation, avatar cropping and Face Bank character suggestions.

## Design principles

- **Local first**: screenshots and materials stay on your own disks; the database
  only stores indexes and relationships and never moves or uploads your files.
  Discovery, classification, indexing and archiving remain fully usable without
  a network or the AI engine.
- **Generic capture**: watches one or more game screenshot directories, detects
  stable new images, supports manual classification and character labeling, and
  archives reliably to local folders or NAS shares. Not tied to any specific
  game, engine or screenshot tool.
- **Optional and replaceable AI**: the recognition engine runs as a separate
  process (system Python in development, an embedded sidecar in the released
  builds); models and runtimes are replaceable. ArcFace weights are
  user-provided and never bundled.

> OCR, image captioning, generic auto-tagging, Embedding, semantic search and
> RAG are future roadmap capabilities and are not implemented yet.

## Installation

**System requirements**: Windows 10/11 x64 (WebView2 runtime required —
preinstalled on Windows 11, provided with Edge on Windows 10; the installer
bootstraps it when missing). No Node, Rust or Python needed.

Download from [Releases](https://github.com/awillheartwu/scene_vault/releases):

| Installer | Contents | Size |
|---|---|---|
| `scene-vault-1.0.0-ai-setup.exe` | App + bundled AI engine (YuNet/SFace/Chinese annotation font) | ~110 MB |
| `scene-vault-1.0.0-setup.exe` | App only (AI engine can be enabled later) | ~6 MB |

**Quick start**: install and launch → create a project → add **screenshot source
directories** (where the game stores captures) and an **archive directory**
(local folder or a NAS `\\...` share) → start the session → screenshots are
discovered automatically; “Import screenshots” batches existing images in the
directory. After labeling a capture as “person”, the AI engine suggests
characters (low-confidence results still require manual confirmation);
unclassified/scene/private classifications stay manual.

## Screenshots

> Images live in `docs/screenshots/`; missing ones render as placeholders.

| Page | Screenshot |
|---|---|
| Home (project overview) | ![Home](screenshots/home.png) |
| Capture | ![Capture](screenshots/capture.png) |
| Workbench | ![Workbench](screenshots/workbench.png) |
| Settings: data safety | ![Data safety](screenshots/settings-data-safety.png) |
| Settings: vision engine | ![Vision engine](screenshots/settings-vision.png) |
| History | ![History](screenshots/history.png) |

## Key features

- **Capture**: multi-directory watching, content-hash deduplication (identical
  screenshots are registered once), stable-write detection, mid-session source
  directory attachment, resume and retry recovery
- **Classification & characters**: person/scene/private/unclassified labels,
  character labeling with rename/merge/representative avatar, global-hotkey
  quick-classify popup
- **Optional AI**: face detection, Face Bank character suggestions, annotated
  image and avatar generation, SFace/ArcFace switching
- **Reliable archiving**: local or NAS, atomic writes, automatic retry with
  backoff, no data loss on NAS disconnects
- **Data safety**: integrity preflight, consistent backups (SHA-256 manifest),
  two-phase restore with rollback, minimal recovery UI when the database cannot
  open, automatic pre-migration backups
- **Scale**: 1.0 gate is 10k captures / 10k-file source directories / 100
  characters per project; server-side pagination for item lists
- **Logging & diagnostics**: structured local logs, resource and engine status
  panels in settings, data inspection and index maintenance

## Building from source

### Frontend and tests

```powershell
pnpm install
npx vitest run          # frontend tests
pnpm build              # type check + production build
start-tauri-dev.bat     # dev mode (vite + cargo run)
```

### Rust backend

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

### Python engine (optional)

```powershell
$env:PYTHONPATH = "python/src"
python -m unittest discover -s python/tests
python -m scene_vault_ai health
```

### Windows release packages

`scripts/release-windows.ps1` builds both NSIS installers in one pass: frontend
build → PyInstaller sidecar → models/fonts packaging → NSIS. Common flags:
`-SkipAI` (small package only), `-AffinityMask` (CPU pinning; 0xF recommended on
13900K-class machines), `-SignCertificatePath` (optional signing). GitHub
Actions (`.github/workflows/release-windows.yml`) builds automatically on `v*`
tags and attaches the installers to the Release.

Environment setup and one-click Windows scripts are described in the
[development guide](DEVELOPMENT.md).

## Repository layout

```text
src/                 Vue frontend
src-tauri/           Tauri/Rust backend, SQLite migrations, packaging config
python/              optional local AI engine (versioned JSON protocol)
scripts/             development and release scripts
docs/                product, architecture, decisions, acceptance and dev docs
AGENTS.md            AI agent working constraints
```

## Documentation

- [Project context](PROJECT_CONTEXT.md)
- [Architecture](ARCHITECTURE.md)
- [Data model](DATA_MODEL.md)
- [Capture Session workflow (incl. Windows acceptance checklist)](CAPTURE_WORKFLOW.md)
- [Face recognition benchmarks](BENCHMARK.md)
- [Resource and storage notes](RESOURCE_USAGE.md)
- [Development guide](DEVELOPMENT.md)
- [Roadmap](ROADMAP.md)
- [Decision records](decisions/)
- [Python AI protocol](../python/README.md)

## Data and licensing

- The database and user data live in the Tauri app data directory (under
  `%APPDATA%` / `%LOCALAPPDATA%`), never inside the repo or media folders;
  uninstalling the app keeps your data.
- The bundled annotation font (得意黑 / Smiley Sans) is SIL OFL-1.1; YuNet/SFace
  models come from opencv_zoo.
- ArcFace weights (`w600k_r50.onnx`) are non-commercial and user-provided; when
  absent the app falls back to the bundled SFace.

## FAQ

- **No AI suggestions appear?** Confirm the settings “vision engine” page shows
  “bundled engine ready”, and that a face is detectable in the screenshot
  (no-face / too-small faces are reported explicitly and never enrolled).
  Anime/game art similarity scores are generally low, so at the default
  threshold 0.5 suggestions may be absent — lower the confidence threshold in
  recognition settings, or switch to ArcFace (better for this art style, but
  requires your own weights).
- **No suggestions after switching recognizers?** Rebuild the face bank for the
  project after switching; awaiting-label captures are automatically
  re-extracted with the active model.
- **First AI call feels slow?** The bundled engine is a single self-extracting
  executable; the first call takes a few seconds to unpack, then the persistent
  worker reuses the models.
- **“Unknown publisher” warning at install?** Installers are not code-signed
  yet; Windows SmartScreen shows a warning — “More info → Run anyway” is safe.
- **How do I share the installers?** Just distribute the two setup files; target
  machines need no Node/Rust/Python. Release links of a private repository are
  member-only; use Forgejo Releases or file hosting instead.
- **Where are the backups?** Settings → Resource & storage → Data safety:
  preflight, backup (SHA-256 manifest), restore (two-phase with rollback) and
  index maintenance; a damaged database opens the minimal recovery UI.
