# AI Assistant — Contextual Command Palette

## Overview

The AI assistant is a **contextual command palette** (not a chatbot) that helps users with database tasks. It uses the generated metadata and SQL database structure of the selected database to provide intelligent SQL generation, formatting, and explanation.

## Interaction Model

### Command Palette (`Ctrl+K`)
A floating centered dialog triggered by hotkey. The user types a natural language question and gets SQL back. Schema context and metadata are injected automatically.

### Prefix Commands
- No prefix = **Natural language to SQL** (default)
- `/explain` = Explain the current editor SQL
- `/optimize` = Optimize the current editor SQL
- `/format` = Reformat the current editor SQL with proper indentation
- `/comment` = Add inline comments to the current editor SQL
- `/docs` = Generate documentation for the loaded schema

### Editor Context Menu
Right-click in the editor shows AI actions:
- AI: Explain Query
- AI: Optimize Query
- AI: Format Query
- AI: Add Comments

These open the palette pre-filled with the selected SQL and fire immediately.

### AI Settings Sidebar (`Ctrl+Shift+A`)
Provider management (add/edit/delete providers, set default) and history of past interactions. The sidebar is separate from the palette.

## Use Cases

1. **SQL Generation**: "show all orders from last month with customer names" → generates the SQL
2. **SQL Formatting**: `/format` → reformats current SQL with indentation and line breaks
3. **SQL Comments**: `/comment` → adds inline comments explaining each section
4. **Query Explanation**: `/explain` → explains what the current SQL does in plain English
5. **Query Optimization**: `/optimize` → suggests an optimized version with explanations

## Architecture

- **Frontend**: `AiCommandPalette.tsx` (floating dialog), `AiPanel.tsx` (settings/history sidebar)
- **Store**: `aiStore.ts` (Zustand) — palette state, streaming, providers, history
- **Backend**: `src-tauri/src/ai/` — prompt building, streaming AI client, provider management
- **Context**: `schemaContext.ts` builds schema+metadata context automatically



### Issues

The AI assistant generates the thinking process as well as the final answer. We need to filter out the thinking process and only show the final answer.

Add openrouter.ai as LLM provider. Use https://openrouter.ai/docs/api/reference/overview as documentation on how to do that.
openrouter has an endpoint (curl https://openrouter.ai/api/v1/models \
     -H "Authorization: Bearer <token>") to list all the available models.
Show that list in the AI settings sidebar, so the user can select one of them.

On a corporate network, the user might encounter a CRYPT_E_NO_REVOCATION_CHECK error when trying to connect to the internet. Add a setting to disable SSL certificate verification for the AI provider.


Keep a prompt history of the last 10 prompts in the AI assitant dialog box.

When a AI prompt is eneterd, always check if it applies to the active query in the editor. If it does, use the active query as context for the AI prompt.

For the Z.ai provider:
Let the user select from 2 endpoints:
- https://api.z.ai/api/coding/paas/v4             (coding endpoint)
- https://api.z.ai/api/paas/v4                    (general endpoint)  

Let the user select from a list of models:
- GLM-5.1
- GLM-5V-Turbo
- GLM-5
- GLM-5-Turbo
- GLM-4.7
- GLM-4.6
- GLM-4.5

Format with ai works great. Can we add a preview that would show the reformatted sql next to the existing in the split pane ?
Green and red colors for the preview?
And a accept or reject button to apply the changes or discard them?


### Inline AI assistance (ghost-text completion)

Separate feature from this command-palette assistant. Full architecture and
implementation plan lives in [`inline-ai.md`](./inline-ai.md).

Short answer: yes, a small local LLM (Qwen2.5-Coder-3B GGUF Q4_K_M) on the
4080 Super, served via a bundled `llama-server` sidecar exposing the FIM
`/infill` endpoint, hits sub-300 ms perceived latency and runs alongside the
existing deterministic Monaco completions with no regressions. See
`inline-ai.md` for the phased plan, performance budget, trigger gating,
cancellation, and settings UX.

In the AI assistant, add the possibility to select the local ai (inline ai) as provider. Only when the inlina ai is enabled.

When the AI assitant generates a query, check if the query is valid sql. If it is not valid sql, let the assistant know and ask it to correct the query.
Also is there a way to validate the query before executing it?