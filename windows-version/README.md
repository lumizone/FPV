# FPV for Windows

Windows port of **FPV Desktop** — a private, local-first desktop app for
AI-driven interactive fiction: local Ollama narration, local
stable-diffusion.cpp illustrations, and SQLite story data in a continuous
story-reader interface.

- Frameless window UI matching the macOS version (Windows window controls,
  edge resize handles, opaque background instead of macOS vibrancy)
- Bundled, self-contained Ollama runtime (one CUDA line, CPU fallback) —
  launched in-process with a pinned `OLLAMA_HOST` and app-scoped model
  store, no user install
- Bundled CPU stable-diffusion.cpp build; accelerated CUDA/ROCm image
  runtime provisioned on first use (per-GPU, SHA-verified,
  sentinel-protected, retry UI)
- Windows Job Object kills orphaned sidecar processes on abnormal exit
- Per-story `Local only` / `Cloud allowed` privacy policy, project backup
  and restore (`.fpv-project`)

## Development

From this directory:

```bash
npm install
npm run package:lock:validate
powershell -ExecutionPolicy Bypass -File scripts/fetch-binaries-win.ps1
npm run tauri -- build --bundles nsis
```

The Windows build and tests run in CI on `windows-latest`; see
[`.github/workflows/windows-build.yml`](.github/workflows/windows-build.yml).
Windows release inputs are pinned separately in
[`src-tauri/binaries-win.lock`](src-tauri/binaries-win.lock); the macOS-only
`binaries.lock` is not used by the Windows fetch or validation path.
The macOS original lives in [`../mac-version`](../mac-version) (separate
repo, `macsrc` remote).

## Stan weryfikacji (Windows)

CI (`windows-latest`) potwierdza: kompilację, testy jednostkowe (backend +
frontend), lint (clippy `-D warnings`), build instalatora NSIS,
uruchomienie binarki i utrzymanie procesu przez 25s.

CI NIE potwierdza: czy GPU faktycznie renderuje obraz przez sd.cpp
(CUDA/ROCm provisioning, sentinel, retry), czy okno wygląda i przeciąga
się poprawnie (bezramkowe UI, resize z krawędzi, Aero-Snap nieadresowany),
czy Ollama faktycznie odpowiada na wiadomość czatu (spawn z env vars,
GPU-runner auto-fetch, Blackwell detect). Te pozostają niepotwierdzone do
pierwszego testu na realnej maszynie Windows.
