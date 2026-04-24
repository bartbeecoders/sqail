# file-save implementation plan

Living plan for implementing `Vibecoding/file-save.md`. Work happens on branch `feature/file-save`, landed as one commit per stage so each is reviewable.

## Defaults (flag to change before building)

- **Encryption passphrase:** optional. Default uses the machine-local keychain secret already used by the app; user can opt into a passphrase per-file. Cross-machine portable only with passphrase.
- **Binary data:** base64 as spec says — only relevant once we include diagram export snapshots; skipped for now.
- **Existing `.sql` files:** `Ctrl+S` behaviour unchanged. Save-As dialog offers `.sql` and `.sqail` filters, with `.sqail` preferred when the tab has connection/history context.

## Stages

### Stage 1 — `.sqail` format foundation
- Cut branch `feature/file-save` off `main`.
- `src/types/sqailFile.ts` — envelope types: `kind: "sql" | "diagram" | "project"`, `version`, `createdAt`, `updatedAt`, payload, optional embedded connection + prompt history.
- Rust commands in `src-tauri/src/` for AES-GCM encrypt/decrypt of sensitive strings. Key derived from per-install secret plus optional user passphrase on save.
- `src/lib/sqail/codec.ts` — serialize/deserialize + version upgrade handling.
- Smoke: round-trip a fixture. No UI changes yet.

### Stage 2 — Per-file AI prompt history
- Today `aiStore.promptHistory` is global. Move into `EditorTab` (`promptHistory: AiHistoryEntry[]`).
- `aiStore` reads/writes the active tab's history via `editorStore`.
- `aiStore.history` (DB-backed, global) unchanged — only the palette's per-tab recency moves.

### Stage 3 — Save/Load SQL tabs as `.sqail`
- Extend `src/lib/fileOps.ts`: if user picks `.sqail`, bundle SQL content + active connection config (password encrypted) + per-tab prompt history.
- On open `.sqail`: decode, load into tab, prompt user before registering the bundled connection (never silently add credentials).
- `.sql` path unchanged.

### Stage 4 — Save/Load diagram tabs as `.sqail`
- Same flow for `EditorTab.kind === "diagram"`: save `DiagramState` + connection + per-tab prompt history.
- On open, recreate the diagram tab with bundled state. Reuse Stage 1 codec/encryption.

### Stage 5 — Projects
- `src/types/project.ts`, `src/stores/projectStore.ts`.
- Sidebar panel "Project" below connections: file tree with open/close.
- Project saved as `.sqail` with `kind: "project"` — array of file entries (inline SQL/diagram payloads) + shared connections + shared settings.
- Open-project restores all files as tabs.

## Status tracking

Progress is tracked in the session task list (TaskCreate/TaskUpdate). This document is the durable plan; the task list is the live checklist.
