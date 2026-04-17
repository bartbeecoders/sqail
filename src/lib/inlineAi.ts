/**
 * Monaco `InlineCompletionsProvider` driven by the local llama.cpp
 * sidecar (see `Vibecoding/inline-ai.md`).
 *
 * Design notes:
 *
 *  * Runs alongside the deterministic `createSqlCompletionProvider` —
 *    that one owns the popup completion list, this one only produces
 *    ghost text. They can't collide because Monaco dispatches them
 *    through different APIs.
 *
 *  * Gating is aggressive. Every LLM call costs ~100–300 ms and ~5 W
 *    of GPU; skipping a call costs nothing. Gates live in
 *    `shouldTrigger()`.
 *
 *  * Debounce is implemented per-editor via a shared timer. A new
 *    keystroke aborts the previous in-flight request (both the local
 *    Promise and the remote llama-server call).
 *
 *  * A persistent module-level event listener dispatches
 *    `inline:chunk` / `inline:done` / `inline:error` to per-request
 *    handlers through a `Map<requestId, handler>`. This avoids the
 *    ~5 ms `listen()` setup cost per keystroke.
 *
 *  * Results are cached in an LRU keyed by
 *    `(prefix-tail hash | suffix-head hash | schema hash | model id)`.
 */

import type { editor, Position, CancellationToken, languages } from "monaco-editor";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { buildInlineContext } from "./inlineContext";
import { hashString, LruCache } from "./lruCache";
import { stripStringsAndComments } from "./sqlCompletions";
import { stripThinkingBlocks } from "./stripThinking";
import { useInlineAiStore } from "../stores/inlineAiStore";
import { useConnectionStore } from "../stores/connectionStore";

const TELEMETRY_PREVIEW_CHARS = 80;

const PREFIX_TAIL_CHARS = 512;
const SUFFIX_HEAD_CHARS = 128;
const CACHE_CAPACITY = 256;

interface CompletionPayload {
  text: string;
  tokens: number;
  ttftMs: number;
  totalMs: number;
  stopReason: string;
}

type ChunkEvent = { requestId: string; chunk: string };
type DoneEvent = { requestId: string; text: string; tokens: number; ttftMs: number; totalMs: number; stopReason: string };
type ErrorEvent = { requestId: string; error: string };

interface PendingHandler {
  onChunk?: (chunk: string) => void;
  resolve: (value: CompletionPayload) => void;
  reject: (reason: Error) => void;
}

// ── Global event dispatch ────────────────────────────────────────────────

const pending = new Map<string, PendingHandler>();

let listenersAttached = false;
let listenerHandles: UnlistenFn[] = [];

async function ensureListeners(): Promise<void> {
  if (listenersAttached) return;
  listenersAttached = true;
  try {
    listenerHandles.push(
      await listen<ChunkEvent>("inline:chunk", (event) => {
        const h = pending.get(event.payload.requestId);
        h?.onChunk?.(event.payload.chunk);
      }),
    );
    listenerHandles.push(
      await listen<DoneEvent>("inline:done", (event) => {
        const { requestId, text, tokens, ttftMs, totalMs, stopReason } = event.payload;
        const h = pending.get(requestId);
        useInlineAiStore.getState().recordCompletion({
          tokens,
          ttftMs,
          totalMs,
          stopReason,
          preview: text.slice(0, TELEMETRY_PREVIEW_CHARS),
        });
        if (h) {
          pending.delete(requestId);
          h.resolve({ text, tokens, ttftMs, totalMs, stopReason });
        }
      }),
    );
    listenerHandles.push(
      await listen<ErrorEvent>("inline:error", (event) => {
        const h = pending.get(event.payload.requestId);
        if (h) {
          pending.delete(event.payload.requestId);
          h.reject(new Error(event.payload.error));
        }
      }),
    );
  } catch (e) {
    console.error("inline AI: failed to attach listeners:", e);
    listenersAttached = false;
  }
}

// ── Shared caching + debounce state ──────────────────────────────────────

const cache = new LruCache<string>(CACHE_CAPACITY);

let debounceTimer: ReturnType<typeof setTimeout> | null = null;
let activeRequestId: string | null = null;

function cancelActive(): void {
  if (activeRequestId) {
    void invoke("inline_complete_cancel", { requestId: activeRequestId }).catch(
      () => {},
    );
    pending.delete(activeRequestId);
    activeRequestId = null;
  }
  if (debounceTimer !== null) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
  }
}

/**
 * Cancel every in-flight inline-completion request and reject pending
 * handlers. Called when the context changes out from under us — sidecar
 * restart, model switch, connection change, etc.
 */
export function cancelAllInlineRequests(reason: string = "context-changed"): void {
  cancelActive();
  // Resolve any remaining handlers with a no-op so callers return
  // empty instead of hanging forever.
  for (const h of pending.values()) {
    try {
      h.resolve({ text: "", tokens: 0, ttftMs: 0, totalMs: 0, stopReason: reason });
    } catch {
      /* ignore */
    }
  }
  pending.clear();
}

// ── Gating ───────────────────────────────────────────────────────────────

interface GateContext {
  model: editor.ITextModel;
  position: Position;
  fullText: string;
  strippedFullText: string;
  cursorOffset: number;
  prefixBefore: string; // text before cursor, unstripped
}

function buildGateContext(
  model: editor.ITextModel,
  position: Position,
): GateContext {
  const fullText = model.getValue();
  const cursorOffset = model.getOffsetAt(position);
  const prefixBefore = fullText.slice(0, cursorOffset);
  const strippedFullText = stripStringsAndComments(fullText);
  return { model, position, fullText, strippedFullText, cursorOffset, prefixBefore };
}

function shouldTrigger(ctx: GateContext, editorInstance: editor.ICodeEditor): boolean {
  // Feature disabled or sidecar not ready.
  const s = useInlineAiStore.getState();
  if (!s.enabled) return false;
  if (s.sidecar.state !== "ready") return false;

  // Non-empty selection.
  const sel = editorInstance.getSelection();
  if (sel && !sel.isEmpty()) return false;

  // Stripped char immediately before cursor tells us whether we're in a
  // string / comment or just after a member-access dot. The deterministic
  // popup provider owns the post-dot completion.
  const strippedChar = ctx.strippedFullText[ctx.cursorOffset - 1] ?? "";
  const rawChar = ctx.prefixBefore.slice(-1);
  if (strippedChar === " " && rawChar !== " " && rawChar !== "\n" && rawChar !== "\t") {
    // character was zapped by the stripper → we're inside a string or
    // comment. Skip.
    return false;
  }
  if (rawChar === ".") return false;

  // Current statement already ends in `;` just before cursor — don't
  // complete past a finished statement.
  const trimmed = ctx.prefixBefore.trimEnd();
  if (trimmed.endsWith(";")) return false;

  return true;
}

// ── Main provider ────────────────────────────────────────────────────────

export function createInlineAiProvider(): languages.InlineCompletionsProvider {
  void ensureListeners();

  return {
    provideInlineCompletions: async (model, position, _context, token) => {
      const editorCtx = buildGateContext(model, position);

      // Look up any editor whose selection we can observe. The selection
      // check is already covered via `model`, so no need to resolve the
      // actual editor instance here; supply a shim that satisfies the
      // gate's API.
      const selectionShim = {
        getSelection: () => null,
      } as unknown as editor.ICodeEditor;
      if (!shouldTrigger(editorCtx, selectionShim)) {
        return { items: [] };
      }

      const { debounceMs, maxTokens, temperature, modelId } =
        useInlineAiStore.getState();

      // Build prompt pieces.
      const prefixTail = editorCtx.prefixBefore.slice(-PREFIX_TAIL_CHARS);
      const suffixHead = editorCtx.fullText
        .slice(editorCtx.cursorOffset)
        .slice(0, SUFFIX_HEAD_CHARS);

      const activeConn = useConnectionStore.getState();
      const driver =
        activeConn.connections.find((c) => c.id === activeConn.activeConnectionId)
          ?.driver ?? "";

      // Inline context surrounds the current statement.
      const currentStatement = extractCurrentStatement(
        editorCtx.strippedFullText,
        editorCtx.cursorOffset,
      );
      const inlineCtx = buildInlineContext(currentStatement, driver);
      const promptPrefix = inlineCtx.prefix
        ? `${inlineCtx.prefix}\n\n${prefixTail}`
        : prefixTail;

      // Cache lookup.
      const cacheKey = [
        hashString(prefixTail),
        hashString(suffixHead),
        inlineCtx.hash,
        modelId,
      ].join(":");
      const cached = cache.get(cacheKey);
      if (cached !== undefined && cached.length > 0) {
        return makeItems(position, cached);
      }

      // Debounce: cancel any earlier pending request, then wait.
      cancelActive();
      try {
        await waitOrCancel(debounceMs, token);
      } catch {
        return { items: [] };
      }

      if (token.isCancellationRequested) return { items: [] };

      // Kick off the backend request.
      let requestId: string;
      try {
        requestId = await invoke<string>("inline_complete_start", {
          prefix: promptPrefix,
          suffix: suffixHead,
          nPredict: maxTokens,
          temperature,
        });
      } catch (e) {
        console.warn("inline_complete_start failed:", e);
        return { items: [] };
      }
      activeRequestId = requestId;

      // Register handler, await completion.
      const payload = await new Promise<CompletionPayload | null>((resolve) => {
        let accumulated = "";
        pending.set(requestId, {
          onChunk: (c) => {
            accumulated += c;
          },
          resolve: (p) => resolve(p),
          reject: (err) => {
            console.warn("inline completion:", err.message);
            resolve(null);
          },
        });

        const onCancel = token.onCancellationRequested(() => {
          onCancel.dispose();
          if (activeRequestId === requestId) {
            cancelActive();
          }
          // Fall through with whatever we accumulated.
          resolve(accumulated.length > 0 ? {
            text: accumulated,
            tokens: 0,
            ttftMs: 0,
            totalMs: 0,
            stopReason: "cancelled",
          } : null);
        });
      });

      if (activeRequestId === requestId) activeRequestId = null;

      if (!payload) return { items: [] };

      const cleaned = truncateAtStop(stripThinkingBlocks(payload.text));
      if (cleaned.trim().length === 0) return { items: [] };

      cache.set(cacheKey, cleaned);
      return makeItems(position, cleaned);
    },

    disposeInlineCompletions: () => {
      /* no-op — our items hold no disposable resources */
    },
  };
}

function makeItems(
  position: Position,
  text: string,
): languages.InlineCompletions {
  return {
    items: [
      {
        insertText: text,
        range: {
          startLineNumber: position.lineNumber,
          startColumn: position.column,
          endLineNumber: position.lineNumber,
          endColumn: position.column,
        },
      },
    ],
  };
}

/**
 * Defence-in-depth stop fence. The Rust completer already truncates at
 * `;` / `\n\n`; this mirrors the logic in JS so a future provider path
 * (external OpenAI-compatible endpoint) that bypasses the Rust fence
 * still can't produce runaway ghost text.
 */
function truncateAtStop(text: string): string {
  let cut = text.length;
  const semi = text.indexOf(";");
  if (semi !== -1 && semi < cut) cut = semi;
  const blank = text.indexOf("\n\n");
  if (blank !== -1 && blank < cut) cut = blank;
  return text.slice(0, cut);
}

function waitOrCancel(ms: number, token: CancellationToken): Promise<void> {
  if (ms <= 0) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      disposeCancelHook();
      resolve();
    }, ms);
    const cancelHook = token.onCancellationRequested(() => {
      clearTimeout(timer);
      disposeCancelHook();
      reject(new Error("cancelled"));
    });
    function disposeCancelHook() {
      cancelHook.dispose();
    }
  });
}

/** Lift the current statement around the cursor from the already-stripped text. */
function extractCurrentStatement(strippedFullText: string, cursorOffset: number): string {
  let start = 0;
  let end = strippedFullText.length;
  for (let i = cursorOffset - 1; i >= 0; i--) {
    if (strippedFullText[i] === ";") {
      start = i + 1;
      break;
    }
  }
  for (let i = cursorOffset; i < strippedFullText.length; i++) {
    if (strippedFullText[i] === ";") {
      end = i;
      break;
    }
  }
  return strippedFullText.slice(start, end);
}

/** Test-only escape hatch — lets tests reset module state between runs. */
export function __resetInlineAiForTests(): void {
  cache.clear();
  pending.clear();
  activeRequestId = null;
  if (debounceTimer !== null) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
  }
  listenerHandles.forEach((f) => f());
  listenerHandles = [];
  listenersAttached = false;
}
