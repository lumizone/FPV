# First Person Viewpoint (FPV)

**First Person Viewpoint (FPV)** is a local-first desktop app for AI-driven interactive fiction. Create a world, decide what you do or say, and let an AI narrator continue the story while characters, places, choices, and consequences persist.

## Status

The repository is being prepared for the first public release, **v0.0.1**.

- **macOS:** source is release-ready. The downloadable build will be signed with Developer ID and notarized by Apple before publication.
- **Windows:** source is included and the port is code-complete. The first Windows download will follow after packaging; it will initially be unsigned.
- **Distribution:** FPV is free. Local play has no account, subscription, license server, or story-credit system.

## What runs locally

- Ollama narration
- stable-diffusion.cpp illustrations
- SQLite worlds, sessions, story state, and continuity
- Project export and restore

Cloud narration and image providers are optional BYOK integrations. Provider credentials remain on the user's machine.

## Repository layout

- `mac-version/` — macOS Tauri application
- `windows-version/` — Windows Tauri application

Both apps use Rust, React, TypeScript, Vite, Tauri 2, SQLite, Ollama, and stable-diffusion.cpp.

## Development

Each platform directory is independently installable:

```bash
cd mac-version # or windows-version
npm install
npm run qa
npm run build
```

`npm run qa` runs linting, TypeScript checks, Vitest, and a production build. Windows-specific packaging requires a Windows release environment.

## Security and releases

Do not commit `.env` files, Apple signing credentials, provider API keys, or downloaded runtime binaries. The repository ignores these paths by default.

Release downloads are only published once their platform-specific verification is complete. The macOS artifact will be signed and Apple-notarized before it is offered publicly.

## License

FPV is licensed under the [GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-or-later).
