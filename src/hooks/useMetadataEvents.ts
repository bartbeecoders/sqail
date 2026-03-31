import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useMetadataStore } from "../stores/metadataStore";
import type {
  MetadataProgress,
  MetadataDone,
  MetadataError,
} from "../types/metadata";

export function useMetadataEvents() {
  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [];

    unlisteners.push(
      listen<MetadataProgress>("metadata:progress", (event) => {
        useMetadataStore.getState().setProgress(event.payload);
      }),
    );

    unlisteners.push(
      listen<MetadataDone>("metadata:done", (event) => {
        useMetadataStore.getState().setDone(event.payload);
      }),
    );

    unlisteners.push(
      listen<MetadataError>("metadata:error", (event) => {
        useMetadataStore.getState().setError(event.payload);
      }),
    );

    return () => {
      unlisteners.forEach((p) => p.then((f) => f()));
    };
  }, []);
}
