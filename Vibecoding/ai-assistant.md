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