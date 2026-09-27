import { Component, useEffect, useMemo, useState, type ErrorInfo, type ReactNode } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Database,
  Loader2,
  Play,
  Power,
  PowerOff,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";

import { cn } from "../lib/utils";
import { useConnectionStore } from "../stores/connectionStore";
import { useInlineAiStore } from "../stores/inlineAiStore";
import { useTrainingStore } from "../stores/trainingStore";
import type { TrainingJob, TrainingPhase } from "../types/training";

/** Shared empty array returned by the log selector when a job hasn't
 *  emitted any lines yet — avoids handing back a fresh [] each render. */
const EMPTY_LOG: string[] = [];

/**
 * Model/connection → LoRA adapter pipeline. Top-level sections:
 *   1. Python environment status (one-shot probe).
 *   2. Composer form (connection, base model, dataset opts, hyperparams).
 *   3. Active + recent training jobs with live progress.
 *   4. Trained model catalogue (activate/delete).
 */
export default function TrainingSettingsTab() {
  return (
    <TabErrorBoundary>
      <TrainingSettingsTabInner />
    </TabErrorBoundary>
  );
}

function TrainingSettingsTabInner() {
  const s = useTrainingStore();
  const connections = useConnectionStore((x) => x.connections);
  const connectedIds = useConnectionStore((x) => x.connectedIds);
  const loadConnections = useConnectionStore((x) => x.loadConnections);
  const inlineModels = useInlineAiStore((x) => x.models);
  const refreshModels = useInlineAiStore((x) => x.refreshModels);

  useEffect(() => {
    loadConnections();
    refreshModels();
    s.checkEnv();
    s.refreshJobs();
    s.refreshTrainedModels();
    let unlisten: (() => void) | null = null;
    s.attachListeners().then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const activeJob = useMemo(
    () => s.jobs.find((j) => isRunning(j.phase)),
    [s.jobs],
  );
  // The most recent job overall — shown in the detail panel regardless
  // of whether it's still running. Important so the error + log stay
  // visible after a failure.
  const latestJob = useMemo(() => s.jobs[0], [s.jobs]);
  const recentJobs = useMemo(() => s.jobs.slice(0, 10), [s.jobs]);

  const selectedConnection = connections.find(
    (c) => c.id === s.form.connectionId,
  );
  const connectionReady =
    selectedConnection && connectedIds.has(selectedConnection.id);
  const baseModel = inlineModels.find((m) => m.id === s.form.baseModelId);
  const canStart =
    !!selectedConnection &&
    !!baseModel &&
    connectionReady &&
    !activeJob &&
    s.env?.torchAvailable === true &&
    s.env?.transformersAvailable === true &&
    s.env?.peftAvailable === true &&
    s.env?.trlAvailable === true;

  return (
    <div className="space-y-5">
      <header>
        <h3 className="text-sm font-semibold">Model Training</h3>
        <p className="text-[11px] text-muted-foreground">
          Fine-tune a base coder model on this database&apos;s schema,
          metadata, and sample data. The result is a LoRA adapter stored
          locally — no data leaves your machine.
        </p>
      </header>

      <EnvPanel />

      <section>
        <SectionHeader>1. Dataset source</SectionHeader>
        <div className="grid grid-cols-[1fr_1fr] gap-3">
          <div>
            <label className="text-[10px] font-medium text-muted-foreground">
              Database connection
            </label>
            <select
              className="input mt-1 h-8 w-full text-xs"
              value={s.form.connectionId}
              onChange={(e) => s.setFormConnection(e.target.value)}
            >
              <option value="">Select…</option>
              {connections.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name} {connectedIds.has(c.id) ? "" : "(not connected)"}
                </option>
              ))}
            </select>
            {selectedConnection && !connectionReady && (
              <div className="mt-1 flex items-center gap-1 text-[11px] text-warning">
                <AlertTriangle size={11} />
                Connect to this database before training.
              </div>
            )}
          </div>
          <div>
            <label className="text-[10px] font-medium text-muted-foreground">
              Base model
            </label>
            <select
              className="input mt-1 h-8 w-full text-xs"
              value={s.form.baseModelId}
              onChange={(e) => s.setFormBaseModel(e.target.value)}
            >
              {inlineModels.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.displayName}
                </option>
              ))}
            </select>
            <div className="mt-1 text-[11px] text-muted-foreground">
              HuggingFace repo is resolved from the inline-AI catalog.
              Base model downloads at training time, not now.
            </div>
          </div>
        </div>
      </section>

      <section>
        <SectionHeader>2. Dataset options</SectionHeader>
        <div className="grid grid-cols-3 gap-3">
          <NumberInput
            label="Sample rows / table"
            value={s.form.options.sampleRows}
            min={0}
            max={50}
            onChange={(v) =>
              s.setFormOptions((o) => ({ ...o, sampleRows: v }))
            }
          />
          <NumberInput
            label="Max tables"
            value={s.form.options.maxTables}
            min={1}
            max={5000}
            onChange={(v) =>
              s.setFormOptions((o) => ({ ...o, maxTables: v }))
            }
          />
          <div />
          <Checkbox
            label="Include generated metadata"
            checked={s.form.options.includeMetadata}
            onChange={(v) =>
              s.setFormOptions((o) => ({ ...o, includeMetadata: v }))
            }
          />
          <Checkbox
            label="Include sample-row examples"
            checked={s.form.options.includeSamples}
            onChange={(v) =>
              s.setFormOptions((o) => ({ ...o, includeSamples: v }))
            }
          />
          <Checkbox
            label="Include join heuristics"
            checked={s.form.options.includeJoins}
            onChange={(v) =>
              s.setFormOptions((o) => ({ ...o, includeJoins: v }))
            }
          />
        </div>
        <div className="mt-3 flex items-center gap-2">
          <button
            type="button"
            className="rounded bg-muted px-3 py-1 text-[11px] font-medium text-foreground hover:bg-accent disabled:opacity-40"
            onClick={() => s.previewDataset()}
            disabled={!connectionReady || s.previewing}
          >
            {s.previewing ? (
              <>
                <Loader2 size={10} className="mr-1 inline animate-spin" />
                Building preview…
              </>
            ) : (
              "Preview dataset"
            )}
          </button>
          {s.previewStats && (
            <span className="text-[11px] text-muted-foreground">
              {s.previewStats.exampleCount} examples ·{" "}
              {s.previewStats.tableCount} tables ·{" "}
              {formatBytes(s.previewStats.sizeBytes)}
            </span>
          )}
          {s.previewError && (
            <span className="flex items-center gap-1 text-[11px] text-destructive">
              <AlertTriangle size={11} />
              {s.previewError}
            </span>
          )}
        </div>
      </section>

      <section>
        <SectionHeader>3. Training hyperparameters</SectionHeader>
        <div className="grid grid-cols-3 gap-3">
          <NumberInput
            label="Epochs"
            value={s.form.hyperparams.epochs}
            min={0.1}
            max={20}
            step={0.1}
            onChange={(v) =>
              s.setFormHyperparams((h) => ({ ...h, epochs: v }))
            }
          />
          <NumberInput
            label="Learning rate"
            value={s.form.hyperparams.learningRate}
            min={1e-6}
            max={1e-2}
            step={1e-5}
            onChange={(v) =>
              s.setFormHyperparams((h) => ({ ...h, learningRate: v }))
            }
          />
          <NumberInput
            label="LoRA rank"
            value={s.form.hyperparams.loraRank}
            min={1}
            max={128}
            step={1}
            onChange={(v) =>
              s.setFormHyperparams((h) => ({ ...h, loraRank: v }))
            }
          />
          <NumberInput
            label="LoRA alpha"
            value={s.form.hyperparams.loraAlpha}
            min={1}
            max={256}
            step={1}
            onChange={(v) =>
              s.setFormHyperparams((h) => ({ ...h, loraAlpha: v }))
            }
          />
          <NumberInput
            label="Max steps (-1 = all)"
            value={s.form.hyperparams.maxSteps}
            min={-1}
            max={100000}
            step={1}
            onChange={(v) =>
              s.setFormHyperparams((h) => ({ ...h, maxSteps: v }))
            }
          />
          <NumberInput
            label="Batch size"
            value={s.form.hyperparams.batchSize}
            min={1}
            max={16}
            step={1}
            onChange={(v) =>
              s.setFormHyperparams((h) => ({ ...h, batchSize: v }))
            }
          />
        </div>
      </section>

      <section>
        <div className="flex items-center justify-between">
          <SectionHeader>4. Run</SectionHeader>
          <button
            type="button"
            onClick={async () => {
              try {
                const id = await s.startJob();
                if (id) s.refreshJobs();
              } catch (e) {
                console.error("[TrainingSettingsTab] startJob click failed", e);
              }
            }}
            disabled={!canStart}
            className="flex items-center gap-1 rounded bg-primary px-3 py-1.5 text-[11px] font-medium text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Play size={12} />
            Start training
          </button>
        </div>
        {latestJob && <JobProgress job={latestJob} />}
        {recentJobs.length === 0 ? (
          <div className="mt-3 rounded-md border border-dashed border-border py-6 text-center text-xs text-muted-foreground">
            No training runs yet.
          </div>
        ) : (
          <div className="mt-3 space-y-1">
            {recentJobs.map((j) => (
              <JobRow key={j.id} job={j} />
            ))}
          </div>
        )}
      </section>

      <TrainedModelsSection />
    </div>
  );
}

function TrainedModelsSection() {
  const models = useTrainingStore((x) => x.trainedModels);
  const activating = useTrainingStore((x) => x.activating);
  const activateError = useTrainingStore((x) => x.activateError);
  const activate = useTrainingStore((x) => x.activateTrainedModel);
  const deactivate = useTrainingStore((x) => x.deactivateTrainedModel);
  const remove = useTrainingStore((x) => x.deleteTrainedModel);
  const sidecar = useInlineAiStore((x) => x.sidecar);

  const activeLoraId =
    (sidecar.state === "ready" || sidecar.state === "starting")
      ? sidecar.activeLoraId ?? null
      : null;
  const sidecarBusy =
    sidecar.state === "starting" || activating !== null;

  return (
    <section>
      <SectionHeader>Trained models</SectionHeader>
      {activeLoraId && (
        <div className="mb-2 flex items-center justify-between rounded-md border border-success/40 bg-success/5 px-3 py-1.5 text-[11px]">
          <span className="flex items-center gap-1 text-success">
            <CheckCircle2 size={11} />
            Active adapter: <code>{activeLoraId}</code>
          </span>
          <button
            onClick={() => deactivate()}
            disabled={sidecarBusy}
            className="flex items-center gap-1 rounded border border-border bg-background px-2 py-1 text-[10px] hover:bg-accent disabled:opacity-40"
          >
            <PowerOff size={10} />
            Deactivate
          </button>
        </div>
      )}
      {activateError && (
        <div className="mb-2 flex items-start gap-1 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-[11px] text-destructive">
          <AlertTriangle size={12} className="mt-0.5 shrink-0" />
          <pre className="whitespace-pre-wrap break-all font-mono leading-relaxed">
            {activateError}
          </pre>
        </div>
      )}
      {models.length === 0 ? (
        <div className="rounded-md border border-dashed border-border py-6 text-center text-xs text-muted-foreground">
          No trained adapters yet. Kick off a training run above.
        </div>
      ) : (
        <div className="space-y-1">
          {models.map((m) => {
            const isActive = activeLoraId === m.id;
            const isPending = activating === m.id;
            return (
              <div
                key={m.id}
                className={cn(
                  "group flex items-center gap-3 rounded-md border px-3 py-2",
                  isActive
                    ? "border-success/40 bg-success/5"
                    : "border-border",
                )}
              >
                <Database
                  size={14}
                  className={
                    isActive ? "text-success" : "text-muted-foreground"
                  }
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 text-xs font-medium">
                    <span>{m.displayName}</span>
                    {isActive && (
                      <span className="rounded bg-success/15 px-1 py-0.5 text-[9px] font-medium text-success">
                        active
                      </span>
                    )}
                    {m.ggufPath && !isActive && (
                      <span
                        className="rounded bg-muted px-1 py-0.5 text-[9px] text-muted-foreground"
                        title={m.ggufPath}
                      >
                        gguf ready
                      </span>
                    )}
                  </div>
                  <div className="text-[10px] text-muted-foreground">
                    {m.exampleCount} examples · {m.tableCount} tables ·{" "}
                    {formatBytes(m.datasetSize)} · base {m.baseModelId} ·{" "}
                    {formatDate(m.createdAt)}
                  </div>
                </div>
                {!isActive && (
                  <button
                    onClick={() => activate(m.id)}
                    disabled={sidecarBusy}
                    className="flex items-center gap-1 rounded border border-border bg-background px-2 py-1 text-[10px] hover:bg-accent disabled:opacity-40"
                    title={
                      m.ggufPath
                        ? "Restart the inline-AI sidecar with this adapter"
                        : "Convert adapter to GGUF and start the inline-AI sidecar with it"
                    }
                  >
                    {isPending ? (
                      <Loader2 size={10} className="animate-spin" />
                    ) : (
                      <Power size={10} />
                    )}
                    {m.ggufPath ? "Activate" : "Convert & activate"}
                  </button>
                )}
                <button
                  onClick={() => remove(m.id)}
                  disabled={isActive || isPending}
                  className="rounded p-1 text-destructive/70 opacity-0 transition-opacity hover:bg-accent hover:text-destructive group-hover:opacity-100 disabled:opacity-0"
                  title={
                    isActive
                      ? "Deactivate before deleting"
                      : "Delete adapter"
                  }
                >
                  <Trash2 size={12} />
                </button>
              </div>
            );
          })}
        </div>
      )}
      <p className="mt-2 text-[10px] text-muted-foreground">
        Activating converts the adapter to GGUF (first time only, 20-60 s) and
        restarts the inline-AI sidecar with <code>--lora</code>. Inline
        completions and the local-AI provider in Ctrl+K will both use it.
      </p>
    </section>
  );
}

// ── Subsections ─────────────────────────────────────────────

function EnvPanel() {
  const env = useTrainingStore((s) => s.env);
  const checking = useTrainingStore((s) => s.envChecking);
  const checkEnv = useTrainingStore((s) => s.checkEnv);

  return (
    <section className="rounded-md border border-border bg-muted/30 p-3">
      <div className="flex items-start justify-between">
        <div>
          <div className="text-xs font-medium">Python environment</div>
          <div className="mt-0.5 text-[11px] text-muted-foreground">
            Training runs through a local Python process. Required packages:{" "}
            <code className="text-[10px]">torch transformers peft trl datasets</code>.
          </div>
        </div>
        <button
          onClick={() => checkEnv()}
          disabled={checking}
          className="rounded bg-muted px-2 py-1 text-[10px] hover:bg-accent disabled:opacity-40"
        >
          {checking ? "Probing…" : "Re-check"}
        </button>
      </div>
      {env ? (
        <div className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-[11px]">
          <EnvRow
            label="Python"
            ok={!!env.pythonPath}
            value={
              env.pythonPath
                ? `${env.pythonPath}${env.pythonVersion ? ` · ${env.pythonVersion}` : ""}`
                : "not found on PATH (set SQAIL_PYTHON to override)"
            }
          />
          <EnvRow label="torch" ok={env.torchAvailable} />
          <EnvRow label="transformers" ok={env.transformersAvailable} />
          <EnvRow label="peft" ok={env.peftAvailable} />
          <EnvRow label="trl" ok={env.trlAvailable} />
          <EnvRow label="datasets" ok={env.datasetsAvailable} />
          <EnvRow
            label="CUDA"
            ok={env.cudaAvailable}
            value={env.cudaDevice ?? (env.cudaAvailable ? "available" : "not available — training will fall back to CPU")}
          />
        </div>
      ) : (
        <div className="mt-2 text-[11px] text-muted-foreground">
          {checking ? "Probing environment…" : "Click Re-check to probe."}
        </div>
      )}
      {env && env.missing.length > 0 && (
        <div className="mt-2 rounded-md border border-warning/30 bg-warning/10 px-2 py-1.5 text-[10px] text-warning">
          Install the missing packages before starting a training run:{" "}
          <code>pip install {env.missing.join(" ")}</code>
        </div>
      )}
    </section>
  );
}

function EnvRow({
  label,
  ok,
  value,
}: {
  label: string;
  ok: boolean;
  value?: string;
}) {
  return (
    <>
      <div className="flex items-center gap-1">
        {ok ? (
          <CheckCircle2 size={11} className="text-success" />
        ) : (
          <AlertTriangle size={11} className="text-warning" />
        )}
        <span>{label}</span>
      </div>
      <span className="text-muted-foreground">{value ?? (ok ? "ok" : "missing")}</span>
    </>
  );
}

function JobProgress({ job }: { job: TrainingJob }) {
  const cancelJob = useTrainingStore((s) => s.cancelJob);
  const pct = Math.round(Math.max(0, Math.min(1, job.progress)) * 100);
  const failed = job.phase === "failed" || !!job.error;
  return (
    <div
      className={cn(
        "mt-3 rounded-md border p-3",
        failed
          ? "border-destructive/40 bg-destructive/5"
          : "border-primary/40 bg-primary/5",
      )}
    >
      <div className="flex items-center justify-between">
        <div>
          <div className="flex items-center gap-2 text-xs font-medium">
            <PhaseBadge phase={job.phase} />
            <span>
              {job.connectionName} → {job.baseModelId}
            </span>
          </div>
          <div className="mt-0.5 text-[11px] text-muted-foreground">
            {job.message ?? "…"}
            {job.step != null && job.totalSteps != null
              ? ` · step ${job.step}/${job.totalSteps}`
              : ""}
            {job.loss != null ? ` · loss ${job.loss.toFixed(4)}` : ""}
          </div>
        </div>
        {isRunning(job.phase) && (
          <button
            onClick={() => cancelJob(job.id)}
            className="flex items-center gap-1 rounded border border-border bg-background px-2 py-1 text-[10px] hover:bg-accent"
          >
            <Square size={10} />
            Cancel
          </button>
        )}
      </div>
      <div className="mt-2 h-1.5 overflow-hidden rounded bg-muted">
        <div
          className={cn(
            "h-full transition-[width]",
            failed ? "bg-destructive" : "bg-primary",
          )}
          style={{ width: `${pct}%` }}
        />
      </div>
      {job.error && (
        <div className="mt-2 flex items-start gap-1 rounded border border-destructive/30 bg-destructive/10 p-2 text-[11px] text-destructive">
          <AlertTriangle size={12} className="mt-0.5 shrink-0" />
          <pre className="whitespace-pre-wrap break-all font-mono leading-relaxed">
            {job.error}
          </pre>
        </div>
      )}
      <JobLogPanel job={job} />
    </div>
  );
}

function JobRow({ job }: { job: TrainingJob }) {
  return (
    <div className="flex items-center gap-2 rounded-md border border-border px-3 py-1.5 text-[11px]">
      <PhaseBadge phase={job.phase} />
      <div className="flex-1 truncate">
        {job.connectionName} → {job.baseModelId}
      </div>
      <span className="text-[10px] text-muted-foreground">
        {formatDate(job.startedAt)}
      </span>
    </div>
  );
}

function JobLogPanel({ job }: { job: TrainingJob }) {
  // Return the stable EMPTY_LOG reference when this job has no logs yet —
  // otherwise `s.logs[jobId] ?? []` would hand back a fresh [] on every
  // store update and re-render this subtree for no reason.
  const log = useTrainingStore((s) => s.logs[job.id] ?? EMPTY_LOG);
  const fetchLog = useTrainingStore((s) => s.fetchLog);
  const [open, setOpen] = useState(false);
  const running = isRunning(job.phase);

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    // When opening the log panel for a terminal job whose in-memory
    // buffer is empty (the store was reloaded, or this job finished
    // before we attached listeners), pull the full log off disk.
    if (next && !running && log.length === 0 && job.logPath) {
      await fetchLog(job);
    }
  };

  const hasLog = log.length > 0 || (!!job.logPath && !running);

  return (
    <div className="mt-2 space-y-1">
      {hasLog && (
        <button
          onClick={toggle}
          className="flex items-center gap-1 text-[10px] text-muted-foreground hover:text-foreground"
        >
          <Terminal size={10} />
          {open
            ? "Hide log"
            : log.length > 0
              ? `View log (${log.length} lines)`
              : "View log"}
        </button>
      )}
      {job.logPath && (
        <div className="text-[10px] text-muted-foreground">
          Log file: <code className="break-all">{job.logPath}</code>
        </div>
      )}
      {open && (
        <pre className="max-h-60 overflow-auto rounded border border-border bg-muted/50 p-2 font-mono text-[10px] leading-relaxed">
          {log.length > 0
            ? log.slice(-200).join("\n")
            : "Reading log file…"}
        </pre>
      )}
    </div>
  );
}

function PhaseBadge({ phase }: { phase: TrainingPhase | string }) {
  const map: Record<string, { label: string; cls: string }> = {
    queued: { label: "queued", cls: "bg-muted text-muted-foreground" },
    preparingDataset: {
      label: "dataset",
      cls: "bg-primary/10 text-primary",
    },
    training: { label: "training", cls: "bg-primary/20 text-primary" },
    saving: { label: "saving", cls: "bg-primary/10 text-primary" },
    completed: {
      label: "completed",
      cls: "bg-success/15 text-success",
    },
    failed: {
      label: "failed",
      cls: "bg-destructive/15 text-destructive",
    },
    cancelled: {
      label: "cancelled",
      cls: "bg-muted text-muted-foreground",
    },
  };
  const info = map[phase] ?? {
    label: String(phase ?? "?"),
    cls: "bg-muted text-muted-foreground",
  };
  return (
    <span
      className={cn(
        "rounded px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wider",
        info.cls,
      )}
    >
      {info.label}
    </span>
  );
}

// ── Local error boundary ───────────────────────────────────

class TabErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[TrainingSettingsTab] render error", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="space-y-3">
          <div className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-xs">
            <div className="mb-1 flex items-center gap-1 font-semibold text-destructive">
              <AlertTriangle size={12} />
              Training tab crashed
            </div>
            <pre className="whitespace-pre-wrap break-all font-mono text-[10px] leading-relaxed text-destructive">
              {this.state.error.message}
              {"\n"}
              {this.state.error.stack}
            </pre>
            <button
              onClick={() => this.setState({ error: null })}
              className="mt-2 rounded bg-muted px-2 py-1 text-[10px] hover:bg-accent"
            >
              Retry
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
      {children}
    </h4>
  );
}

function NumberInput({
  label,
  value,
  onChange,
  min,
  max,
  step,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
}) {
  return (
    <div>
      <label className="text-[10px] font-medium text-muted-foreground">
        {label}
      </label>
      <input
        type="number"
        className="input mt-1 h-8 w-full text-xs"
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}

function Checkbox({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-[11px]">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

function isRunning(p: TrainingPhase): boolean {
  return (
    p === "queued" ||
    p === "preparingDataset" ||
    p === "training" ||
    p === "saving"
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}
