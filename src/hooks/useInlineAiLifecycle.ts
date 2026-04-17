import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

import { cancelAllInlineRequests } from "../lib/inlineAi";
import { useConnectionStore } from "../stores/connectionStore";
import {
  useInlineAiStore,
  type DownloadState,
  type SidecarState,
} from "../stores/inlineAiStore";

/**
 * Mount once (App level) to:
 *  - subscribe to `inline:sidecar-status` / `inline:model-download-progress`
 *    events from the Rust backend
 *  - refresh model catalog + status on boot
 *  - honour the `autoStart` setting (start the sidecar if enabled).
 */
export function useInlineAiLifecycle(): void {
  useEffect(() => {
    const store = useInlineAiStore.getState();

    const unlisteners: Array<Promise<() => void>> = [];

    unlisteners.push(
      listen<SidecarState>("inline:sidecar-status", (event) => {
        useInlineAiStore.getState().applySidecarStatus(event.payload);
      }),
    );

    unlisteners.push(
      listen<{ id: string } & DownloadState>(
        "inline:model-download-progress",
        (event) => {
          useInlineAiStore.getState().applyDownloadProgress(event.payload);
        },
      ),
    );

    void store.refreshModels();
    void store.refreshStatus();

    if (store.enabled && store.autoStart) {
      // Best-effort — the sidecar may fail if the model isn't downloaded,
      // the settings UI surfaces the error from the state machine.
      void store.startSidecar().catch(() => {
        /* already surfaced via sidecar state */
      });
    }

    // Cancel any in-flight completions on meaningful state changes —
    // the old suggestion is against a stale context and would be
    // either wrong or confusing if it arrived late.
    const unsubSidecar = useInlineAiStore.subscribe((state, prev) => {
      if (
        prev.sidecar.state === "ready" &&
        state.sidecar.state !== "ready"
      ) {
        cancelAllInlineRequests("sidecar-not-ready");
      }
      if (state.modelId !== prev.modelId) {
        cancelAllInlineRequests("model-changed");
      }
      if (state.enabled !== prev.enabled && !state.enabled) {
        cancelAllInlineRequests("disabled");
      }
    });

    const unsubConn = useConnectionStore.subscribe((state, prev) => {
      if (state.activeConnectionId !== prev.activeConnectionId) {
        cancelAllInlineRequests("connection-changed");
      }
    });

    return () => {
      unsubSidecar();
      unsubConn();
      unlisteners.forEach((p) => p.then((f) => f()).catch(() => {}));
    };
  }, []);
}
