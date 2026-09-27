import { useState, useEffect, useCallback } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { Download, X, RefreshCw, CheckCircle2 } from "lucide-react";
import { cn } from "../lib/utils";

type UpdateState =
  | { status: "idle" }
  | { status: "available"; update: Update; version: string }
  | { status: "downloading"; progress: number }
  | { status: "ready" }
  | { status: "error"; message: string };

export default function UpdateChecker() {
  const [state, setState] = useState<UpdateState>({ status: "idle" });
  const [dismissed, setDismissed] = useState(false);

  const checkForUpdate = useCallback(async () => {
    try {
      const update = await check();
      if (update) {
        setState({
          status: "available",
          update,
          version: update.version,
        });
      }
    } catch (err) {
      console.warn("Update check failed:", err);
    }
  }, []);

  useEffect(() => {
    // Check after a short delay so the app finishes loading first
    const timer = setTimeout(checkForUpdate, 5000);
    return () => clearTimeout(timer);
  }, [checkForUpdate]);

  const handleDownloadAndInstall = useCallback(async () => {
    if (state.status !== "available") return;
    const { update } = state;

    try {
      let totalBytes = 0;
      let downloadedBytes = 0;

      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            totalBytes = event.data.contentLength ?? 0;
            setState({ status: "downloading", progress: 0 });
            break;
          case "Progress":
            downloadedBytes += event.data.chunkLength;
            setState({
              status: "downloading",
              progress: totalBytes > 0 ? (downloadedBytes / totalBytes) * 100 : 0,
            });
            break;
          case "Finished":
            setState({ status: "ready" });
            break;
        }
      });

      setState({ status: "ready" });
    } catch (err) {
      setState({
        status: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  }, [state]);

  const handleRelaunch = useCallback(async () => {
    await relaunch();
  }, []);

  if (dismissed || state.status === "idle") return null;

  return (
    <div className="flex items-center gap-2 border-b border-border bg-primary/10 px-4 py-1.5">
      {state.status === "available" && (
        <>
          <Download size={13} className="shrink-0 text-primary" />
          <span className="text-[11px] text-foreground">
            Version <strong>{state.version}</strong> is available.
          </span>
          <button
            onClick={handleDownloadAndInstall}
            className="ml-1 rounded bg-primary px-2 py-0.5 text-[10px] font-medium text-primary-foreground hover:bg-primary/90"
          >
            Update now
          </button>
          <button
            onClick={() => setDismissed(true)}
            className="ml-auto rounded p-0.5 text-muted-foreground hover:text-foreground"
          >
            <X size={12} />
          </button>
        </>
      )}

      {state.status === "downloading" && (
        <>
          <RefreshCw size={13} className="shrink-0 animate-spin text-primary" />
          <span className="text-[11px] text-foreground">
            Downloading update...
          </span>
          <div className="ml-2 h-1.5 w-32 overflow-hidden rounded-full bg-muted">
            <div
              className={cn(
                "h-full rounded-full bg-primary transition-all",
                state.progress === 0 && "animate-pulse",
              )}
              style={{ width: `${Math.max(state.progress, 2)}%` }}
            />
          </div>
          <span className="text-[10px] text-muted-foreground">
            {Math.round(state.progress)}%
          </span>
        </>
      )}

      {state.status === "ready" && (
        <>
          <CheckCircle2 size={13} className="shrink-0 text-green-500" />
          <span className="text-[11px] text-foreground">
            Update installed. Restart to apply.
          </span>
          <button
            onClick={handleRelaunch}
            className="ml-1 rounded bg-primary px-2 py-0.5 text-[10px] font-medium text-primary-foreground hover:bg-primary/90"
          >
            Restart now
          </button>
          <button
            onClick={() => setDismissed(true)}
            className="ml-auto rounded p-0.5 text-muted-foreground hover:text-foreground"
          >
            <X size={12} />
          </button>
        </>
      )}

      {state.status === "error" && (
        <>
          <span className="text-[11px] text-destructive">
            Update failed: {state.message}
          </span>
          <button
            onClick={() => { setState({ status: "idle" }); checkForUpdate(); }}
            className="ml-1 text-[10px] text-primary hover:underline"
          >
            Retry
          </button>
          <button
            onClick={() => setDismissed(true)}
            className="ml-auto rounded p-0.5 text-muted-foreground hover:text-foreground"
          >
            <X size={12} />
          </button>
        </>
      )}
    </div>
  );
}
