import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useAiStore } from "../stores/aiStore";
import type { AiStreamChunk, AiStreamError } from "../types/ai";

export function useAiStream() {
  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [];

    unlisteners.push(
      listen<AiStreamChunk>("ai:stream-chunk", (event) => {
        useAiStore.getState().appendChunk(event.payload.requestId, event.payload.chunk);
      }),
    );

    unlisteners.push(
      listen<AiStreamChunk>("ai:stream-done", (event) => {
        useAiStore.getState().finishStream(event.payload.requestId, event.payload.chunk);
      }),
    );

    unlisteners.push(
      listen<AiStreamError>("ai:stream-error", (event) => {
        useAiStore.getState().setStreamError(event.payload.requestId, event.payload.error);
      }),
    );

    return () => {
      unlisteners.forEach((p) => p.then((f) => f()));
    };
  }, []);
}
