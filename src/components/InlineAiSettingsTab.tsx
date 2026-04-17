import { Download, Play, RotateCcw, Square, Trash2, Loader2, AlertTriangle, CheckCircle2 } from "lucide-react";
import { useEffect, useMemo } from "react";

import { cn } from "../lib/utils";
import {
  useInlineAiStore,
  type CompletionTelemetry,
  type ModelListItem,
  type DownloadState,
} from "../stores/inlineAiStore";
import { useTrainingStore } from "../stores/trainingStore";

/** Shared shape for the model + binary progress bars. */
interface ProgressSnapshot {
  downloaded: number;
  total: number;
  phase: string;
  error?: string;
}

/**
 * Settings tab for the inline-AI (ghost-text) feature. Renders:
 *   1. Enable switch + contextual onboarding banner.
 *   2. Sidecar status panel with Start/Stop/Restart.
 *   3. Model picker with per-entry download / cancel / delete.
 *   4. Tuning knobs.
 */
export default function InlineAiSettingsTab() {
  const s = useInlineAiStore();
  // Trained adapters come from the training store (single source of
  // truth — `training:done` events refresh it at app level).
  const trainedModels = useTrainingStore((x) => x.trainedModels);
  const refreshTrainedModels = useTrainingStore(
    (x) => x.refreshTrainedModels,
  );
  // Also re-pull the list every time this tab mounts so a user who
  // just finished training sees the adapter immediately — no need to
  // switch tabs.
  useEffect(() => {
    void refreshTrainedModels();
  }, [refreshTrainedModels]);
  const selectedModel = s.models.find((m) => m.id === s.modelId);

  return (
    <div className="space-y-5">
      <header>
        <h3 className="text-sm font-semibold">Inline AI Completion</h3>
        <p className="text-[11px] text-muted-foreground">
          Ghost-text SQL suggestions powered by a local llama.cpp sidecar — no
          cloud calls, nothing leaves your machine.
        </p>
      </header>

      {/* Enable switch */}
      <section className="rounded-md border border-border bg-muted/30 p-3">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-xs font-medium">Enable inline completion</div>
            <div className="mt-0.5 text-[11px] text-muted-foreground">
              Press <kbd className="rounded border border-border bg-background px-1 text-[10px]">Tab</kbd> to accept a suggestion, <kbd className="rounded border border-border bg-background px-1 text-[10px]">Esc</kbd> to dismiss.
            </div>
          </div>
          <Toggle checked={s.enabled} onChange={() => s.toggleEnabled()} />
        </div>
        {s.enabled && !s.binary.installed && s.binary.supported && (
          <BinaryBanner />
        )}
        {s.enabled && selectedModel && !selectedModel.downloaded && (
          <OnboardingBanner model={selectedModel} />
        )}
      </section>

      {/* Sidecar runtime (llama-server binary) */}
      <section>
        <SectionHeader>Sidecar runtime</SectionHeader>
        <SidecarRuntimePanel />
      </section>

      {/* Sidecar status */}
      <section>
        <SectionHeader>Sidecar</SectionHeader>
        <SidecarPanel />
      </section>

      {/* Model picker */}
      <section>
        <SectionHeader>Models</SectionHeader>
        <div className="space-y-2">
          {s.models.length === 0 ? (
            <div className="rounded-md border border-dashed border-border py-6 text-center text-xs text-muted-foreground">
              Loading model catalog…
            </div>
          ) : (
            s.models.map((m) => (
              <ModelRow
                key={m.id}
                model={m}
                selected={m.id === s.modelId && s.trainedModelId === null}
                progress={s.downloads[m.id]}
              />
            ))
          )}
          {trainedModels.length > 0 && (
            <>
              <div className="pt-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                Your trained adapters
              </div>
              {trainedModels.map((tm) => (
                <TrainedModelRow
                  key={tm.id}
                  adapter={tm}
                  selected={s.trainedModelId === tm.id}
                />
              ))}
            </>
          )}
        </div>
      </section>

      {/* Tuning */}
      <section>
        <SectionHeader>Tuning</SectionHeader>
        <div className="space-y-3">
          <Row label="Debounce (ms)">
            <input
              type="number"
              min={0}
              max={1000}
              value={s.debounceMs}
              onChange={(e) =>
                s.updateSetting("debounceMs", Number(e.target.value))
              }
              className="input w-24 text-center"
            />
          </Row>
          <Row label="Max tokens per completion">
            <input
              type="number"
              min={4}
              max={512}
              value={s.maxTokens}
              onChange={(e) =>
                s.updateSetting("maxTokens", Number(e.target.value))
              }
              className="input w-24 text-center"
            />
          </Row>
          <Row label="Temperature">
            <input
              type="number"
              min={0}
              max={2}
              step={0.05}
              value={s.temperature}
              onChange={(e) =>
                s.updateSetting("temperature", Number(e.target.value))
              }
              className="input w-24 text-center"
            />
          </Row>
          <Row label="Context window (tokens)">
            <div className="flex flex-col items-end gap-0.5">
              <select
                value={s.ctxSize}
                onChange={(e) =>
                  s.updateSetting("ctxSize", Number(e.target.value))
                }
                className="input w-28"
              >
                <option value={2048}>2048</option>
                <option value={4096}>4096</option>
                <option value={8192}>8192</option>
                <option value={16384}>16384</option>
                <option value={32768}>32768</option>
              </select>
              <span className="text-[10px] text-muted-foreground">
                Restart sidecar to apply · higher = more VRAM
              </span>
            </div>
          </Row>
          <Row label="Auto-start sidecar on launch">
            <Toggle
              checked={s.autoStart}
              onChange={(v) => s.updateSetting("autoStart", v)}
            />
          </Row>
          <Row label="Force CPU (disable GPU offload)">
            <Toggle
              checked={s.cpuOnly}
              onChange={(v) => s.updateSetting("cpuOnly", v)}
            />
          </Row>
          <Row label="Use as default AI provider">
            <Toggle
              checked={s.useAsDefaultProvider}
              onChange={(v) => s.updateSetting("useAsDefaultProvider", v)}
            />
          </Row>
          <Row label="Show latency telemetry">
            <Toggle
              checked={s.devMode}
              onChange={(v) => s.updateSetting("devMode", v)}
            />
          </Row>
        </div>
      </section>

      {s.devMode && (
        <section>
          <SectionHeader>Recent completions</SectionHeader>
          <TelemetryTable rows={s.telemetry} onClear={() => s.clearTelemetry()} />
        </section>
      )}
    </div>
  );
}

function TelemetryTable({
  rows,
  onClear,
}: {
  rows: CompletionTelemetry[];
  onClear: () => void;
}) {
  if (rows.length === 0) {
    return (
      <div className="rounded-md border border-dashed border-border p-3 text-[11px] text-muted-foreground">
        No completions recorded yet. Type in the editor to see timing data here.
      </div>
    );
  }
  return (
    <div className="space-y-2">
      <div className="flex justify-end">
        <SmallBtn onClick={onClear}>
          <Trash2 size={11} /> Clear
        </SmallBtn>
      </div>
      <div className="overflow-hidden rounded-md border border-border">
        <table className="w-full text-[11px]">
          <thead className="bg-muted/40 text-left text-muted-foreground">
            <tr>
              <th className="px-2 py-1 font-medium">Time</th>
              <th className="px-2 py-1 font-medium">TTFT</th>
              <th className="px-2 py-1 font-medium">Total</th>
              <th className="px-2 py-1 font-medium">Tok</th>
              <th className="px-2 py-1 font-medium">Stop</th>
              <th className="px-2 py-1 font-medium">Preview</th>
            </tr>
          </thead>
          <tbody className="font-mono">
            {rows.map((r, i) => (
              <tr key={`${r.at}-${i}`} className="border-t border-border/50">
                <td className="px-2 py-1 text-muted-foreground">
                  {formatTime(r.at)}
                </td>
                <td className="px-2 py-1">{r.ttftMs} ms</td>
                <td className="px-2 py-1">{r.totalMs} ms</td>
                <td className="px-2 py-1">{r.tokens}</td>
                <td className="px-2 py-1 text-muted-foreground">{r.stopReason}</td>
                <td className="max-w-[300px] truncate px-2 py-1" title={r.preview}>
                  {r.preview || <span className="text-muted-foreground">(empty)</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function formatTime(epochMs: number): string {
  const d = new Date(epochMs);
  return `${d.getHours().toString().padStart(2, "0")}:${d
    .getMinutes()
    .toString()
    .padStart(2, "0")}:${d.getSeconds().toString().padStart(2, "0")}`;
}

// ── Onboarding ────────────────────────────────────────────────────────────

function BinaryBanner() {
  const supported = useInlineAiStore((s) => s.binary.supported);
  const progress = useInlineAiStore((s) => s.binaryDownload);
  const downloadBinary = useInlineAiStore((s) => s.downloadBinary);
  const busy =
    !!progress &&
    progress.phase !== "completed" &&
    progress.phase !== "error" &&
    progress.phase !== "cancelled";

  if (!supported) {
    return (
      <div className="mt-3 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-[11px] text-amber-900 dark:text-amber-200">
        Runtime download isn't available for this platform yet. Point at your own
        <code className="mx-1 rounded bg-amber-500/20 px-1 font-mono">llama-server</code>
        via <code className="mx-1 rounded bg-amber-500/20 px-1 font-mono">SQAIL_LLAMA_SERVER_PATH</code>.
      </div>
    );
  }

  return (
    <div className="mt-3 rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-[11px] font-medium text-amber-900 dark:text-amber-200">
            The llama-server sidecar binary isn't installed yet.
          </div>
          <div className="mt-0.5 text-[11px] text-amber-800/80 dark:text-amber-200/80">
            Downloads once from the official llama.cpp release — cached locally.
          </div>
        </div>
        <button
          onClick={() => downloadBinary()}
          disabled={busy}
          className="flex shrink-0 items-center gap-1 rounded-md bg-amber-500 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-amber-600 disabled:opacity-50"
        >
          {busy ? <Loader2 size={12} className="animate-spin" /> : <Download size={12} />}
          {busy ? "Downloading" : "Download runtime"}
        </button>
      </div>
      {busy && progress && <ProgressBar state={progress} className="mt-2" />}
    </div>
  );
}

function SidecarRuntimePanel() {
  const binary = useInlineAiStore((s) => s.binary);
  const progress = useInlineAiStore((s) => s.binaryDownload);
  const downloadBinary = useInlineAiStore((s) => s.downloadBinary);
  const cancelDownload = useInlineAiStore((s) => s.cancelBinaryDownload);
  const deleteBinary = useInlineAiStore((s) => s.deleteBinary);

  const busy =
    !!progress &&
    progress.phase !== "completed" &&
    progress.phase !== "error" &&
    progress.phase !== "cancelled";

  if (!binary.supported) {
    return (
      <div className="rounded-md border border-border bg-muted/20 p-3 text-[11px] text-muted-foreground">
        No bundled runtime for this platform. Set
        <code className="mx-1 rounded bg-muted px-1 font-mono">SQAIL_LLAMA_SERVER_PATH</code>
        to your own <code className="rounded bg-muted px-1 font-mono">llama-server</code> build.
      </div>
    );
  }

  return (
    <div className="space-y-2 rounded-md border border-border bg-muted/20 p-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-xs">
          {binary.installed ? (
            <CheckCircle2 size={13} className="text-emerald-500" />
          ) : (
            <AlertTriangle size={13} className="text-amber-500" />
          )}
          <div>
            <div className="font-medium">
              {binary.installed ? "Installed" : "Not installed"}
              {binary.releaseTag && (
                <span className="ml-1 text-[10px] font-normal text-muted-foreground">
                  {binary.releaseTag}
                </span>
              )}
            </div>
            {binary.installed && binary.path && (
              <div className="truncate text-[10px] text-muted-foreground" title={binary.path}>
                {binary.path}
              </div>
            )}
          </div>
        </div>
        <div className="flex shrink-0 gap-1">
          {binary.installed ? (
            <SmallBtn onClick={() => deleteBinary()} variant="danger">
              <Trash2 size={11} /> Delete
            </SmallBtn>
          ) : busy ? (
            <SmallBtn onClick={() => cancelDownload()} variant="danger">
              <Square size={11} /> Cancel
            </SmallBtn>
          ) : (
            <SmallBtn onClick={() => downloadBinary()}>
              <Download size={11} /> Download
            </SmallBtn>
          )}
        </div>
      </div>
      {busy && progress && <ProgressBar state={progress} />}
      {!busy && progress && progress.phase === "error" && (
        <div className="text-[10px] text-destructive">{progress.error}</div>
      )}
    </div>
  );
}

function OnboardingBanner({ model }: { model: ModelListItem }) {
  const progress = useInlineAiStore((s) => s.downloads[model.id]);
  const downloadModel = useInlineAiStore((s) => s.downloadModel);
  const busy = progress && !isTerminal(progress.phase);

  return (
    <div className="mt-3 rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-[11px] font-medium text-amber-900 dark:text-amber-200">
            The default model <span className="font-semibold">{model.displayName}</span> isn't downloaded yet.
          </div>
          <div className="mt-0.5 text-[11px] text-amber-800/80 dark:text-amber-200/80">
            ~{formatBytes(model.sizeBytes)} — downloads over HTTPS and is cached locally.
          </div>
        </div>
        <button
          onClick={() => downloadModel(model.id)}
          disabled={busy}
          className="flex shrink-0 items-center gap-1 rounded-md bg-amber-500 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-amber-600 disabled:opacity-50"
        >
          {busy ? <Loader2 size={12} className="animate-spin" /> : <Download size={12} />}
          {busy ? "Downloading" : "Download now"}
        </button>
      </div>
      {busy && progress && <ProgressBar state={progress} className="mt-2" />}
    </div>
  );
}

// ── Sidecar panel ─────────────────────────────────────────────────────────

function SidecarPanel() {
  const sidecar = useInlineAiStore((s) => s.sidecar);
  const start = useInlineAiStore((s) => s.startSidecar);
  const stop = useInlineAiStore((s) => s.stopSidecar);

  const running = sidecar.state === "ready" || sidecar.state === "starting";
  const dotColor = useMemo(() => {
    switch (sidecar.state) {
      case "ready":    return "bg-emerald-500";
      case "starting": return "bg-amber-500 animate-pulse";
      case "error":    return "bg-red-500";
      default:         return "bg-muted-foreground/40";
    }
  }, [sidecar.state]);

  const label = useMemo(() => {
    switch (sidecar.state) {
      case "stopped":  return "Stopped";
      case "starting": return `Starting ${sidecar.modelId}…`;
      case "ready":    return `Running on 127.0.0.1:${sidecar.port} (${sidecar.modelId})`;
      case "error":    return `Error: ${sidecar.message}`;
    }
  }, [sidecar]);

  return (
    <div className="rounded-md border border-border bg-muted/20 p-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 text-xs">
          <span className={cn("h-2 w-2 rounded-full", dotColor)} />
          <span>{label}</span>
        </div>
        <div className="flex gap-1">
          {running ? (
            <>
              <SmallBtn onClick={() => stop().then(() => start())} disabled={sidecar.state === "starting"}>
                <RotateCcw size={11} /> Restart
              </SmallBtn>
              <SmallBtn onClick={() => stop()} variant="danger">
                <Square size={11} /> Stop
              </SmallBtn>
            </>
          ) : (
            <SmallBtn onClick={() => start()}>
              <Play size={11} /> Start
            </SmallBtn>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Model row ─────────────────────────────────────────────────────────────

function ModelRow({
  model,
  selected,
  progress,
}: {
  model: ModelListItem;
  selected: boolean;
  progress?: DownloadState;
}) {
  const actions = useInlineAiStore();
  const busy = progress && !isTerminal(progress.phase);

  return (
    <div
      className={cn(
        "rounded-md border p-3 text-xs",
        selected
          ? "border-primary/50 bg-primary/5"
          : "border-border bg-muted/10",
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <label className="flex flex-1 cursor-pointer items-start gap-2">
          <input
            type="radio"
            name="inline-ai-model"
            checked={selected}
            onChange={() => {
              actions.updateSetting("modelId", model.id);
              // Picking the raw base clears any active adapter so the
              // next sidecar restart goes back to the unadorned model.
              actions.updateSetting("trainedModelId", null);
            }}
            className="mt-0.5"
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="font-medium">{model.displayName}</span>
              <TierBadge tier={model.tier} />
              {model.downloaded && (
                <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                  Downloaded
                </span>
              )}
            </div>
            <div className="mt-0.5 text-[11px] text-muted-foreground">
              {formatBytes(model.sizeBytes)} · min VRAM ~{model.minVramMib} MiB
              {model.downloaded && ` · on disk ${formatBytes(model.diskSize)}`}
            </div>
          </div>
        </label>
        <div className="flex shrink-0 flex-col gap-1">
          {!model.downloaded && !busy && (
            <SmallBtn onClick={() => actions.downloadModel(model.id)}>
              <Download size={11} /> Download
            </SmallBtn>
          )}
          {busy && (
            <SmallBtn onClick={() => actions.cancelDownload(model.id)} variant="danger">
              <Square size={11} /> Cancel
            </SmallBtn>
          )}
          {model.downloaded && !busy && (
            <SmallBtn onClick={() => actions.deleteModel(model.id)} variant="danger">
              <Trash2 size={11} /> Delete
            </SmallBtn>
          )}
        </div>
      </div>
      {progress && !isTerminalSuccess(progress.phase) && (
        <ProgressBar state={progress} className="mt-2" />
      )}
    </div>
  );
}

function TrainedModelRow({
  adapter,
  selected,
}: {
  adapter: import("../types/training").TrainedModel;
  selected: boolean;
}) {
  const actions = useInlineAiStore();
  const baseExists = actions.models.some((m) => m.id === adapter.baseModelId);
  const baseDownloaded =
    actions.models.find((m) => m.id === adapter.baseModelId)?.downloaded ??
    false;

  return (
    <div
      className={cn(
        "rounded-md border p-3 text-xs",
        selected
          ? "border-primary/50 bg-primary/5"
          : "border-border bg-muted/10",
      )}
    >
      <label className="flex cursor-pointer items-start gap-2">
        <input
          type="radio"
          name="inline-ai-model"
          checked={selected}
          disabled={!baseExists || !baseDownloaded}
          onChange={() => {
            // Selecting an adapter implies using its base model — keep
            // the two fields in sync so the sidecar boots the right
            // pair on restart.
            actions.updateSetting("modelId", adapter.baseModelId);
            actions.updateSetting("trainedModelId", adapter.id);
          }}
          className="mt-0.5"
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-medium">{adapter.displayName}</span>
            <span className="rounded bg-indigo-500/15 px-1.5 py-0.5 text-[10px] font-medium text-indigo-700 dark:text-indigo-300">
              adapter
            </span>
            {adapter.ggufPath && (
              <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                GGUF ready
              </span>
            )}
          </div>
          <div className="mt-0.5 text-[11px] text-muted-foreground">
            base <code>{adapter.baseModelId}</code> ·{" "}
            {adapter.exampleCount} examples · {adapter.tableCount} tables
            {!baseDownloaded && (
              <span className="ml-1 text-warning">
                · base model not downloaded
              </span>
            )}
          </div>
        </div>
      </label>
    </div>
  );
}

function TierBadge({ tier }: { tier: ModelListItem["tier"] }) {
  const label = tier === "default" ? "default" : tier === "performance" ? "perf" : "lite";
  const color =
    tier === "default"
      ? "bg-primary/15 text-primary"
      : tier === "performance"
        ? "bg-violet-500/15 text-violet-700 dark:text-violet-300"
        : "bg-sky-500/15 text-sky-700 dark:text-sky-300";
  return (
    <span className={cn("rounded px-1.5 py-0.5 text-[10px] font-medium capitalize", color)}>
      {label}
    </span>
  );
}

// ── Small primitives ──────────────────────────────────────────────────────

function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
      {children}
    </h4>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[1fr_auto] items-center gap-4">
      <span className="text-xs text-foreground">{label}</span>
      <div className="flex justify-end">{children}</div>
    </div>
  );
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-5 w-9 rounded-full transition-colors",
        checked ? "bg-primary" : "bg-muted-foreground/30",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white transition-transform shadow-sm",
          checked && "translate-x-4",
        )}
      />
    </button>
  );
}

function SmallBtn({
  children,
  onClick,
  disabled,
  variant = "default",
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  variant?: "default" | "danger";
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium transition-colors disabled:opacity-50",
        variant === "danger"
          ? "border border-border bg-background text-destructive hover:bg-destructive/10"
          : "border border-border bg-background hover:bg-accent",
      )}
    >
      {children}
    </button>
  );
}

function ProgressBar({
  state,
  className,
}: {
  state: ProgressSnapshot;
  className?: string;
}) {
  const pct =
    state.total > 0 ? Math.min(100, Math.round((state.downloaded / state.total) * 100)) : 0;
  return (
    <div className={cn("space-y-1", className)}>
      <div className="h-1.5 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full transition-all",
            state.phase === "error"
              ? "bg-destructive"
              : state.phase === "completed"
                ? "bg-emerald-500"
                : "bg-primary",
          )}
          style={{ width: `${pct}%` }}
        />
      </div>
      <div className="flex justify-between text-[10px] text-muted-foreground">
        <span className="capitalize">
          {state.phase}
          {state.error && ` — ${state.error}`}
        </span>
        <span>
          {formatBytes(state.downloaded)} / {formatBytes(state.total)}
          {state.total > 0 && ` · ${pct}%`}
        </span>
      </div>
    </div>
  );
}

// ── Helpers ───────────────────────────────────────────────────────────────

function isTerminal(phase: DownloadState["phase"]): boolean {
  return phase === "completed" || phase === "error" || phase === "cancelled";
}

function isTerminalSuccess(phase: DownloadState["phase"]): boolean {
  return phase === "completed";
}

function formatBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  let x = n;
  while (x >= 1024 && i < units.length - 1) {
    x /= 1024;
    i++;
  }
  return `${x >= 10 || i === 0 ? x.toFixed(0) : x.toFixed(1)} ${units[i]}`;
}
