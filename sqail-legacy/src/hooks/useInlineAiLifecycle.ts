import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

import { cancelAllInlineRequests } from "../lib/inlineAi";
import { useConnectionStore } from "../stores/connectionStore";
import {
  useInlineAiStore,
  type BinaryDownloadState,
  type DownloadState,
  type SidecarState,
} from "../stores/inlineAiStore";
import { useTrainingStore } from "../stores/trainingStore";

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

    unlisteners.push(
      listen<BinaryDownloadState>(
        "inline:binary-download-progress",
        (event) => {
          useInlineAiStore
            .getState()
            .applyBinaryDownloadProgress(event.payload);
        },
      ),
    );

    void store.refreshModels();
    void store.refreshStatus();
    void store.refreshBinary();

    // Boot the training store at app-level too: fetch the adapter
    // catalog up front and subscribe to `training:*` events so a job
    // that completes while the Training tab is closed still updates
    // the Inline AI model list's "Your trained adapters" section.
    const trainingStore = useTrainingStore.getState();
    void trainingStore.refreshTrainedModels();
    let detachTraining: (() => void) | null = null;
    void trainingStore.attachListeners().then((unlisten) => {
      detachTraining = unlisten;
    });

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
      if (state.trainedModelId !== prev.trainedModelId) {
        cancelAllInlineRequests("adapter-changed");
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
      detachTraining?.();
    };
  }, []);
}
