# FPV Automated QA

Run the deterministic QA suite from `app/`:

```bash
npm run qa
```

The runner writes a timestamped report to `qa-reports/` and checks:

- TypeScript compilation and production Vite build
- frontend unit tests
- Rust backend tests, including the full story lifecycle
- SQLite migrations, sessions, codex, Story State, branch and cleanup behavior
- i18n parity and missing active keys
- shell syntax and whitespace errors
- absence of Supabase client/URL references in shipped source

Run the packaged macOS GUI smoke test separately:

```bash
npm run qa:gui
```

This verifies the signed app bundle and confirms that the FPV window opens. The
terminal may need macOS Accessibility permission for System Events.

Run a real multi-turn story against the installed Ollama model:

```bash
npm run qa:story
```

Optional environment variables: `FPV_QA_MODEL`, `FPV_QA_WORLD`,
`FPV_QA_TURNS`, `OLLAMA_URL`, and `FPV_DB`.

## Manual Coverage

Real-model QA is intentionally separate because it is slow and hardware
dependent. It must cover:

- 50-100 turn local and cloud stories
- model download, resume, cancellation and missing-weight errors
- scene images, covers, retry, same-seed and new variation
- branch, regenerate, undo and session restart
- HTML, PDF, Markdown and JSON export with hostile text
- Keychain migration and BYOK provider requests
- offline mode and network inspection
- VoiceOver/keyboard navigation
- clean-machine Gatekeeper and Apple notarization
