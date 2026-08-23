<div align="center">
  <img src=".github/assets/fpv-logo.png" alt="First Person Viewpoint logo" width="112" />

  # First Person Viewpoint

  **Your choices move the story forward. The world remembers.**

  A free, local-first desktop app for AI-driven interactive fiction.

  [![License: AGPL-3.0-or-later](https://img.shields.io/badge/License-AGPL--3.0--or--later-d9ff72?style=flat-square&labelColor=111111)](LICENSE)
  ![Local-first](https://img.shields.io/badge/Local--first-Your%20machine%2C%20your%20story-71d9ca?style=flat-square&labelColor=111111)
  ![Platforms](https://img.shields.io/badge/Platforms-macOS%20%2B%20Windows-d9ff72?style=flat-square&labelColor=111111)
  ![Status](https://img.shields.io/badge/Status-v0.0.1%20in%20release%20preparation-8ca1b3?style=flat-square&labelColor=111111)
</div>

![A lone traveler facing a lightning-lit mountain — a Worldlines story image](.github/assets/fpv-worldlines-hero.jpg)

## What is FPV?

**First Person Viewpoint (FPV)** lets you inhabit the protagonist of an evolving story. Build or choose a world, write what you do or say, and let the narrator carry the scene forward. Characters, locations, inventory, relationships, and consequences persist between turns.

FPV is built around ownership: your stories live on your computer, local play needs no account, and the source is available under **AGPL-3.0-or-later**.

| You bring | FPV carries forward |
| --- | --- |
| An action, line of dialogue, or direction | A narrated response in the world’s voice |
| A world, character, or rule | Persistent lore, relationships, quests, and facts |
| A choice | Consequences that can matter many scenes later |

## Why local-first?

- **No account for local play** — no subscription, license server, or story-credit system.
- **Your data stays yours** — worlds, sessions, story state, and provider credentials remain on your machine.
- **Bring your own model** — run narration with Ollama and illustrations with stable-diffusion.cpp locally.
- **Cloud is optional** — supported narration and image providers use your own API key; keys are stored locally.
- **Move or inspect your work** — export and restore a project, then inspect the source that runs it.

## Release status

| Platform | Source | First downloadable release |
| --- | --- | --- |
| **macOS** | Included | v0.0.1 will be Developer ID signed and notarized by Apple before publication. |
| **Windows** | Included | Packaging follows next; the initial Windows build will be unsigned. |

There are no release binaries in this repository yet. Downloads will be published only after the platform-specific verification is complete.

## Included in the source

- Local Ollama narration
- Local stable-diffusion.cpp image generation
- SQLite worlds, sessions, story state, and continuity
- Project export and restore
- Optional BYOK cloud narration and image providers
- Seven interface locales: English, Polish, German, Spanish, Japanese, Korean, and Chinese

## Repository layout

```text
FPV2.0/
├── mac-version/      # macOS Tauri application
├── windows-version/  # Windows Tauri application
└── LICENSE           # AGPL-3.0-or-later
```

Both platform apps use **Tauri 2**, **Rust**, **React**, **TypeScript**, **Vite**, **SQLite**, **Ollama**, and **stable-diffusion.cpp**.

## Run it from source

### Prerequisites

- Node.js and npm
- Rust toolchain
- Platform-native build dependencies for Tauri
- Native runtime binaries fetched or built for your platform — these are deliberately not committed

### macOS

```bash
cd mac-version
npm install
npm run qa
npm run build
```

For a native package, fetch the pinned Ollama runtime first. `scripts/fetch-binaries.sh` validates the download; it also expects a separately verified `sd-cli` binary through `FPV_SD_CLI_SOURCE` and `FPV_SD_CLI_SHA256`.

### Windows

```bash
cd windows-version
npm install
npm run qa
npm run build
```

Windows packaging requires a Windows release environment. The bundled runtimes are downloaded during its packaging workflow and are not versioned in Git.

## Security and distribution

Never commit `.env` files, Apple signing credentials, provider API keys, or downloaded runtime binaries. The root `.gitignore` excludes them by default.

The macOS artifact will be signed and notarized before it is offered publicly. Windows signing is planned after the initial release.

## License

FPV is licensed under the [GNU Affero General Public License v3.0 or later](LICENSE).
