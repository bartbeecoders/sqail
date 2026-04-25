import { useCallback, useEffect, useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  GitBranch,
  GitCommitVertical,
  GitPullRequestArrow,
  Download,
  Upload,
  FolderGit2,
  Loader2,
  RefreshCcw,
  Camera,
  FileDiff,
  Cable,
  Trash2,
  Link as LinkIcon,
  X,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useProjectStore } from "../stores/projectStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useGitStore } from "../stores/gitStore";
import { useEditorStore } from "../stores/editorStore";
import { useToastStore } from "../stores/toastStore";
import { listen } from "@tauri-apps/api/event";
import type { GitCredentials, GitFileChange, GitFileDiff, PullResult } from "../types/git";
import { invoke } from "@tauri-apps/api/core";

type Tab = "status" | "branches" | "remotes";

/**
 * Sidebar panel for the git integration. Assumes `project` exists — the parent
 * ProjectPanel gates rendering on that.
 */
export default function GitPanel() {
  const project = useProjectStore((s) => s.project);
  const setGitConfig = useProjectStore((s) => s.setGitConfig);

  const gitStore = useGitStore();
  const { status, branches, remotes, loading, error } = gitStore;
  const connections = useConnectionStore((s) => s.connections);
  const showToast = useToastStore((s) => s.show);
  const pushToast = useCallback(
    (t: { kind: "info" | "warning" | "error" | "success"; message: string }) =>
      showToast(t.message, t.kind),
    [showToast],
  );

  const [tab, setTab] = useState<Tab>("status");
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [fileDiff, setFileDiff] = useState<GitFileDiff | null>(null);
  const [commitMessage, setCommitMessage] = useState("");
  const [committing, setCommitting] = useState(false);
  const [creds, setCreds] = useState<GitCredentials>({});
  const [busyOp, setBusyOp] = useState<string | null>(null);

  const gitConfig = project?.git;
  const repoPath = gitConfig?.repoPath ?? null;
  const linkedConnectionId = gitConfig?.connectionId;
  const linkedConnection = connections.find((c) => c.id === linkedConnectionId);

  // Re-open the repo whenever the project's git config changes.
  useEffect(() => {
    if (!repoPath) {
      gitStore.reset();
      return;
    }
    gitStore.openRepo(repoPath).catch(() => {
      /* error already surfaced in store */
    });
  }, [repoPath]); // eslint-disable-line react-hooks/exhaustive-deps

  // Refresh the selected file's diff whenever the working tree changes.
  useEffect(() => {
    if (!repoPath || !selectedFile) {
      setFileDiff(null);
      return;
    }
    gitStore
      .fileDiff(repoPath, selectedFile)
      .then(setFileDiff)
      .catch(() => setFileDiff(null));
  }, [repoPath, selectedFile, status]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleLinkExisting = useCallback(async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    if (!selected) return;
    const path = selected as string;
    try {
      await gitStore.openRepo(path);
      setGitConfig({ ...(gitConfig ?? {}), repoPath: path });
    } catch (e) {
      pushToast({ kind: "error", message: `Open repo failed: ${e}` });
    }
  }, [gitConfig, setGitConfig, gitStore, pushToast]);

  const handleInit = useCallback(async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    if (!selected) return;
    const path = selected as string;
    try {
      await gitStore.initRepo(path);
      setGitConfig({ ...(gitConfig ?? {}), repoPath: path });
    } catch (e) {
      pushToast({ kind: "error", message: `Init failed: ${e}` });
    }
  }, [gitConfig, setGitConfig, gitStore, pushToast]);

  const handleUnlink = useCallback(() => {
    setGitConfig(undefined);
    gitStore.reset();
    setSelectedFile(null);
    setFileDiff(null);
  }, [setGitConfig, gitStore]);

  const handleSnapshot = useCallback(async () => {
    if (!repoPath || !linkedConnectionId) {
      pushToast({
        kind: "error",
        message: "Link a connection to this git repo before snapshotting.",
      });
      return;
    }
    setBusyOp("snapshot");
    try {
      const summary = await gitStore.snapshotSchema(
        repoPath,
        linkedConnectionId,
        gitConfig?.includeRoutines ?? true,
      );
      setGitConfig({
        ...(gitConfig ?? { repoPath }),
        repoPath,
        lastSnapshotAt: new Date().toISOString(),
      });
      pushToast({
        kind: "success",
        message: `Snapshot wrote ${summary.filesWritten} file${summary.filesWritten === 1 ? "" : "s"}${summary.removed.length ? `, removed ${summary.removed.length}` : ""}.`,
      });
    } catch (e) {
      pushToast({ kind: "error", message: `Snapshot failed: ${e}` });
    } finally {
      setBusyOp(null);
    }
  }, [repoPath, linkedConnectionId, gitConfig, gitStore, setGitConfig, pushToast]);

  const handleStage = useCallback(
    async (paths: string[]) => {
      if (!repoPath) return;
      try {
        await gitStore.stage(repoPath, paths);
      } catch (e) {
        pushToast({ kind: "error", message: `Stage failed: ${e}` });
      }
    },
    [repoPath, gitStore, pushToast],
  );

  const handleUnstage = useCallback(
    async (paths: string[]) => {
      if (!repoPath) return;
      try {
        await gitStore.unstage(repoPath, paths);
      } catch (e) {
        pushToast({ kind: "error", message: `Unstage failed: ${e}` });
      }
    },
    [repoPath, gitStore, pushToast],
  );

  const handleCommit = useCallback(async () => {
    if (!repoPath || !commitMessage.trim()) return;
    setCommitting(true);
    try {
      await gitStore.commit(repoPath, commitMessage.trim());
      setCommitMessage("");
      pushToast({ kind: "success", message: "Committed." });
    } catch (e) {
      pushToast({ kind: "error", message: `Commit failed: ${e}` });
    } finally {
      setCommitting(false);
    }
  }, [repoPath, commitMessage, gitStore, pushToast]);

  const handleFetch = useCallback(async () => {
    const remote = gitConfig?.defaultRemote ?? remotes[0]?.name ?? "origin";
    if (!repoPath) return;
    setBusyOp("fetch");
    try {
      await gitStore.fetch(repoPath, remote, creds);
      pushToast({ kind: "success", message: `Fetched ${remote}.` });
    } catch (e) {
      pushToast({ kind: "error", message: `Fetch failed: ${e}` });
    } finally {
      setBusyOp(null);
    }
  }, [repoPath, remotes, gitConfig?.defaultRemote, gitStore, creds, pushToast]);

  const handlePull = useCallback(async () => {
    const remote = gitConfig?.defaultRemote ?? remotes[0]?.name ?? "origin";
    if (!repoPath) return;
    setBusyOp("pull");
    try {
      const result: PullResult = await gitStore.pull(repoPath, remote, creds);
      if (result.kind === "fast-forward" && result.changedFiles.length > 0) {
        pushToast({
          kind: "success",
          message: `${result.message} ${result.changedFiles.length} file(s) changed. Use "Generate migration" to produce SQL.`,
        });
      } else {
        pushToast({
          kind: result.kind === "merge-required" ? "error" : "success",
          message: result.message,
        });
      }
    } catch (e) {
      pushToast({ kind: "error", message: `Pull failed: ${e}` });
    } finally {
      setBusyOp(null);
    }
  }, [repoPath, remotes, gitConfig?.defaultRemote, gitStore, creds, pushToast]);

  const handlePush = useCallback(async () => {
    const remote = gitConfig?.defaultRemote ?? remotes[0]?.name ?? "origin";
    const branch = status?.branch;
    if (!repoPath || !branch) {
      pushToast({ kind: "error", message: "No current branch to push." });
      return;
    }
    setBusyOp("push");
    try {
      await gitStore.push(repoPath, remote, branch, creds);
      pushToast({ kind: "success", message: `Pushed ${branch} → ${remote}.` });
    } catch (e) {
      pushToast({ kind: "error", message: `Push failed: ${e}` });
    } finally {
      setBusyOp(null);
    }
  }, [repoPath, remotes, gitConfig?.defaultRemote, status?.branch, gitStore, creds, pushToast]);

  const handleGenerateMigration = useCallback(async () => {
    if (!repoPath || !linkedConnection) {
      pushToast({
        kind: "error",
        message: "Link a connection before generating migrations.",
      });
      return;
    }
    setBusyOp("migration");
    try {
      const files = await gitStore.changedFiles(repoPath);
      if (files.length === 0) {
        pushToast({ kind: "info", message: "No changes to migrate." });
        return;
      }
      const diffText = files
        .map(
          (f) =>
            `## ${f.path}\n\n=== OLD (HEAD) ===\n${f.oldContent}\n\n=== NEW (working tree) ===\n${f.newContent}\n\n=== PATCH ===\n${f.patch}`,
        )
        .join("\n\n----\n\n");

      // Open a new tab to stream the migration into.
      const title = `migration-${new Date().toISOString().slice(0, 10)}.sql`;
      useEditorStore.getState().addTabWithContent(title, "-- Generating migration…\n");
      const tab = useEditorStore.getState().getActiveTab();
      if (!tab) return;
      useEditorStore.getState().setConnectionId(tab.id, linkedConnection.id);

      // Kick off streaming.
      const requestId = await invoke<string>("ai_generate_migration_script", {
        diffText,
        driver: linkedConnection.driver,
        providerId: null,
      });

      let buffer = "";
      const unsubs: (() => void)[] = [];
      unsubs.push(
        await listen<{ requestId: string; chunk: string }>("ai:stream-chunk", (ev) => {
          if (ev.payload.requestId !== requestId) return;
          buffer += ev.payload.chunk;
          useEditorStore.getState().setContent(tab.id, buffer);
        }),
      );
      unsubs.push(
        await listen<{ requestId: string }>("ai:stream-done", (ev) => {
          if (ev.payload.requestId !== requestId) return;
          pushToast({
            kind: "success",
            message: "Migration script ready — review before executing.",
          });
          unsubs.forEach((u) => u());
        }),
      );
      unsubs.push(
        await listen<{ requestId: string; error: string }>("ai:stream-error", (ev) => {
          if (ev.payload.requestId !== requestId) return;
          pushToast({
            kind: "error",
            message: `Migration generation failed: ${ev.payload.error}`,
          });
          unsubs.forEach((u) => u());
        }),
      );
    } catch (e) {
      pushToast({ kind: "error", message: `Migration generation failed: ${e}` });
    } finally {
      setBusyOp(null);
    }
  }, [repoPath, linkedConnection, gitStore, pushToast]);

  // ── Rendering ────────────────────────────────────────────────────
  if (!project) {
    return null;
  }

  if (!repoPath) {
    return (
      <div className="flex flex-col gap-2 border-t border-border p-3">
        <p className="text-[11px] text-muted-foreground">
          Link this project to a local git repository to snapshot its schema as SQL files
          and commit/push changes.
        </p>
        <button
          onClick={handleLinkExisting}
          className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-[11px] hover:bg-accent"
        >
          <FolderGit2 size={12} /> Link existing repository…
        </button>
        <button
          onClick={handleInit}
          className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-[11px] hover:bg-accent"
        >
          <GitBranch size={12} /> Init new repository…
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col border-t border-border">
      {/* Repo header */}
      <div className="flex items-center gap-1 border-b border-border px-2 py-1.5">
        <FolderGit2 size={12} className="text-muted-foreground" />
        <div className="min-w-0 flex-1 text-[11px]">
          <div className="truncate font-medium" title={repoPath}>
            {repoPath.split(/[/\\]/).pop()}
          </div>
          <div className="truncate text-[9px] text-muted-foreground">
            {status?.branch ? `on ${status.branch}` : "no branch"}
            {status?.upstream ? ` — upstream ${status.upstream}` : ""}
            {status && (status.ahead || status.behind)
              ? ` — ↑${status.ahead} ↓${status.behind}`
              : ""}
          </div>
        </div>
        <button
          onClick={() => gitStore.refresh(repoPath)}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          title="Refresh status"
          disabled={loading}
        >
          {loading ? <Loader2 size={11} className="animate-spin" /> : <RefreshCcw size={11} />}
        </button>
        <button
          onClick={handleUnlink}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          title="Unlink repository from project"
        >
          <X size={11} />
        </button>
      </div>

      {/* Connection link bar */}
      <div className="flex items-center gap-1 border-b border-border px-2 py-1.5">
        <LinkIcon size={11} className="text-muted-foreground" />
        <span className="text-[10px] text-muted-foreground">Connection:</span>
        <select
          value={linkedConnectionId ?? ""}
          onChange={(e) =>
            setGitConfig({
              ...(gitConfig ?? { repoPath }),
              repoPath,
              connectionId: e.target.value || undefined,
            })
          }
          className="min-w-0 flex-1 rounded border border-border bg-background px-1 py-0.5 text-[10px]"
        >
          <option value="">(unlinked)</option>
          {connections.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      </div>

      {/* Tabs */}
      <div className="flex shrink-0 border-b border-border">
        {(
          [
            { id: "status" as const, label: "Status", icon: FileDiff },
            { id: "branches" as const, label: "Branches", icon: GitBranch },
            { id: "remotes" as const, label: "Remotes", icon: Cable },
          ]
        ).map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={cn(
              "flex flex-1 items-center justify-center gap-1 py-1.5 text-[10px] font-medium transition-colors",
              tab === id
                ? "border-b-2 border-primary text-primary"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon size={11} />
            {label}
          </button>
        ))}
      </div>

      {error && (
        <div className="border-b border-destructive/30 bg-destructive/10 px-2 py-1 text-[10px] text-destructive">
          {error}
        </div>
      )}

      {tab === "status" && (
        <StatusView
          files={status?.files ?? []}
          selected={selectedFile}
          onSelect={setSelectedFile}
          onStage={(p) => handleStage([p])}
          onUnstage={(p) => handleUnstage([p])}
          onSnapshot={handleSnapshot}
          snapshotDisabled={!linkedConnectionId || busyOp === "snapshot"}
          commitMessage={commitMessage}
          onCommitMessage={setCommitMessage}
          onCommit={handleCommit}
          committing={committing}
          onFetch={handleFetch}
          onPull={handlePull}
          onPush={handlePush}
          onGenerateMigration={handleGenerateMigration}
          busyOp={busyOp}
          remote={gitConfig?.defaultRemote ?? remotes[0]?.name ?? "origin"}
          hasRemote={remotes.length > 0}
        />
      )}
      {tab === "branches" && <BranchesView branches={branches} current={status?.branch ?? null} />}
      {tab === "remotes" && (
        <RemotesView
          remotes={remotes}
          defaultRemote={gitConfig?.defaultRemote}
          onDefaultChange={(name) =>
            setGitConfig({ ...(gitConfig ?? { repoPath }), repoPath, defaultRemote: name })
          }
          onAdd={(name, url) => gitStore.setRemote(repoPath, name, url)}
          onRemove={(name) => gitStore.removeRemote(repoPath, name)}
          creds={creds}
          onCredsChange={setCreds}
        />
      )}

      {/* Selected file diff */}
      {tab === "status" && selectedFile && fileDiff && (
        <FileDiffPreview diff={fileDiff} onClose={() => setSelectedFile(null)} />
      )}
    </div>
  );
}

// ── sub-components ───────────────────────────────────────────────────

function StatusView(props: {
  files: GitFileChange[];
  selected: string | null;
  onSelect: (p: string | null) => void;
  onStage: (path: string) => void;
  onUnstage: (path: string) => void;
  onSnapshot: () => void;
  snapshotDisabled: boolean;
  commitMessage: string;
  onCommitMessage: (s: string) => void;
  onCommit: () => void;
  committing: boolean;
  onFetch: () => void;
  onPull: () => void;
  onPush: () => void;
  onGenerateMigration: () => void;
  busyOp: string | null;
  remote: string;
  hasRemote: boolean;
}) {
  const {
    files,
    selected,
    onSelect,
    onStage,
    onUnstage,
    onSnapshot,
    snapshotDisabled,
    commitMessage,
    onCommitMessage,
    onCommit,
    committing,
    onFetch,
    onPull,
    onPush,
    onGenerateMigration,
    busyOp,
    remote,
    hasRemote,
  } = props;

  const hasStaged = useMemo(() => files.some((f) => f.staged), [files]);

  return (
    <div className="flex flex-col">
      {/* Action row */}
      <div className="flex flex-wrap gap-1 border-b border-border p-1.5">
        <ActionButton
          icon={Camera}
          label="Snapshot"
          onClick={onSnapshot}
          disabled={snapshotDisabled}
          busy={busyOp === "snapshot"}
          title="Write DDL for every schema object to the repo"
        />
        <ActionButton
          icon={Download}
          label="Fetch"
          onClick={onFetch}
          disabled={!hasRemote}
          busy={busyOp === "fetch"}
        />
        <ActionButton
          icon={GitPullRequestArrow}
          label="Pull"
          onClick={onPull}
          disabled={!hasRemote}
          busy={busyOp === "pull"}
        />
        <ActionButton
          icon={Upload}
          label={`Push → ${remote}`}
          onClick={onPush}
          disabled={!hasRemote}
          busy={busyOp === "push"}
        />
        <ActionButton
          icon={GitCommitVertical}
          label="Gen migration"
          onClick={onGenerateMigration}
          busy={busyOp === "migration"}
          title="Generate SQL migration from diffs via AI"
        />
      </div>

      {/* File list */}
      <div className="max-h-56 overflow-y-auto">
        {files.length === 0 ? (
          <p className="p-3 text-[11px] text-muted-foreground">
            Working tree is clean.
          </p>
        ) : (
          <ul>
            {files.map((f) => {
              const isSelected = selected === f.path;
              return (
                <li
                  key={f.path}
                  className={cn(
                    "group flex items-center gap-1 px-2 py-1 text-[11px] hover:bg-accent/50",
                    isSelected && "bg-accent",
                  )}
                >
                  <span
                    className={cn(
                      "inline-block w-3 shrink-0 text-center text-[9px] font-bold",
                      f.staged ? "text-success" : "text-muted-foreground",
                    )}
                    title={
                      f.staged
                        ? f.hasUnstaged
                          ? "Staged (with unstaged changes)"
                          : "Staged"
                        : "Unstaged"
                    }
                  >
                    {f.staged ? (f.hasUnstaged ? "±" : "✓") : "·"}
                  </span>
                  <span
                    className={cn(
                      "inline-block w-4 shrink-0 text-center text-[9px] font-bold",
                      f.kind === "new"
                        ? "text-success"
                        : f.kind === "deleted"
                          ? "text-destructive"
                          : f.kind === "conflicted"
                            ? "text-destructive"
                            : "text-warning",
                    )}
                    title={f.kind}
                  >
                    {f.kind[0].toUpperCase()}
                  </span>
                  <button
                    onClick={() => onSelect(isSelected ? null : f.path)}
                    className="min-w-0 flex-1 truncate text-left"
                    title={f.path}
                  >
                    {f.path}
                  </button>
                  <div className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                    {f.staged ? (
                      <button
                        onClick={() => onUnstage(f.path)}
                        className="rounded px-1 text-[9px] font-medium hover:bg-background/80"
                        title="Unstage"
                      >
                        −
                      </button>
                    ) : (
                      <button
                        onClick={() => onStage(f.path)}
                        className="rounded px-1 text-[9px] font-medium hover:bg-background/80"
                        title="Stage"
                      >
                        +
                      </button>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {/* Commit */}
      <div className="border-t border-border p-1.5">
        <textarea
          rows={2}
          value={commitMessage}
          onChange={(e) => onCommitMessage(e.target.value)}
          placeholder="Commit message…"
          className="w-full resize-none rounded border border-border bg-background px-2 py-1 text-[11px] outline-none focus:border-primary"
        />
        <button
          onClick={onCommit}
          disabled={!commitMessage.trim() || !hasStaged || committing}
          className={cn(
            "mt-1 flex w-full items-center justify-center gap-1 rounded-md bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground",
            (!commitMessage.trim() || !hasStaged || committing) &&
              "cursor-not-allowed opacity-50",
          )}
        >
          {committing ? <Loader2 size={11} className="animate-spin" /> : <GitCommitVertical size={11} />}
          Commit {hasStaged ? "" : "(nothing staged)"}
        </button>
      </div>
    </div>
  );
}

function BranchesView({
  branches,
  current,
}: {
  branches: string[];
  current: string | null;
}) {
  return (
    <div className="max-h-56 overflow-y-auto">
      {branches.length === 0 ? (
        <p className="p-3 text-[11px] text-muted-foreground">
          No branches yet. Create an initial commit to materialise the default branch.
        </p>
      ) : (
        <ul>
          {branches.map((b) => (
            <li
              key={b}
              className={cn(
                "flex items-center gap-1 px-2 py-1 text-[11px]",
                b === current && "bg-accent font-medium",
              )}
            >
              <GitBranch size={11} className="text-muted-foreground" />
              <span className="truncate">{b}</span>
              {b === current && <span className="text-[9px] text-muted-foreground">current</span>}
            </li>
          ))}
        </ul>
      )}
      <p className="p-3 text-[10px] text-muted-foreground">
        Branch create/switch/delete arrives in Phase 2. Use your git client for now.
      </p>
    </div>
  );
}

function RemotesView({
  remotes,
  defaultRemote,
  onDefaultChange,
  onAdd,
  onRemove,
  creds,
  onCredsChange,
}: {
  remotes: { name: string; url: string }[];
  defaultRemote?: string;
  onDefaultChange: (name: string) => void;
  onAdd: (name: string, url: string) => Promise<void>;
  onRemove: (name: string) => Promise<void>;
  creds: GitCredentials;
  onCredsChange: (c: GitCredentials) => void;
}) {
  const [name, setName] = useState("origin");
  const [url, setUrl] = useState("");

  return (
    <div className="flex flex-col gap-2 p-2">
      <div>
        <div className="mb-1 text-[10px] font-semibold uppercase text-muted-foreground">
          Remotes
        </div>
        {remotes.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">No remotes configured.</p>
        ) : (
          <ul className="space-y-0.5">
            {remotes.map((r) => (
              <li key={r.name} className="flex items-center gap-1 text-[11px]">
                <input
                  type="radio"
                  name="default-remote"
                  checked={(defaultRemote ?? remotes[0]?.name) === r.name}
                  onChange={() => onDefaultChange(r.name)}
                  className="shrink-0"
                />
                <span className="w-16 shrink-0 truncate font-medium">{r.name}</span>
                <span className="min-w-0 flex-1 truncate text-muted-foreground" title={r.url}>
                  {r.url}
                </span>
                <button
                  onClick={() => onRemove(r.name)}
                  className="rounded p-1 hover:bg-background/80"
                  title="Remove remote"
                >
                  <Trash2 size={10} className="text-muted-foreground hover:text-destructive" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="border-t border-border pt-2">
        <div className="mb-1 text-[10px] font-semibold uppercase text-muted-foreground">
          Add / update remote
        </div>
        <div className="flex flex-col gap-1">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="name"
            className="rounded border border-border bg-background px-2 py-1 text-[11px]"
          />
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://github.com/you/repo.git"
            className="rounded border border-border bg-background px-2 py-1 text-[11px]"
          />
          <button
            onClick={async () => {
              if (!name.trim() || !url.trim()) return;
              await onAdd(name.trim(), url.trim());
              setUrl("");
            }}
            className="rounded-md bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground hover:bg-primary/90"
          >
            Save remote
          </button>
        </div>
      </div>

      <div className="border-t border-border pt-2">
        <div className="mb-1 text-[10px] font-semibold uppercase text-muted-foreground">
          HTTPS credentials
        </div>
        <div className="flex flex-col gap-1">
          <input
            value={creds.username ?? ""}
            onChange={(e) => onCredsChange({ ...creds, username: e.target.value })}
            placeholder="username (or leave empty for token-only)"
            className="rounded border border-border bg-background px-2 py-1 text-[11px]"
          />
          <input
            type="password"
            value={creds.password ?? ""}
            onChange={(e) => onCredsChange({ ...creds, password: e.target.value })}
            placeholder="personal access token / password"
            className="rounded border border-border bg-background px-2 py-1 text-[11px]"
          />
          <p className="text-[9px] text-muted-foreground">
            Credentials are held only in memory for this session.
          </p>
        </div>
      </div>
    </div>
  );
}

function FileDiffPreview({
  diff,
  onClose,
}: {
  diff: GitFileDiff;
  onClose: () => void;
}) {
  return (
    <div className="border-t border-border">
      <div className="flex items-center justify-between bg-muted/30 px-2 py-1 text-[10px]">
        <span className="truncate font-medium" title={diff.path}>
          {diff.path}
        </span>
        <button onClick={onClose} className="rounded p-0.5 hover:bg-accent">
          <X size={10} />
        </button>
      </div>
      <pre className="max-h-56 overflow-auto bg-background px-2 py-1 font-mono text-[10px] leading-snug">
        {diff.patch || "(no hunks)"}
      </pre>
    </div>
  );
}

function ActionButton({
  icon: Icon,
  label,
  onClick,
  disabled,
  busy,
  title,
}: {
  icon: typeof Camera;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  busy?: boolean;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || busy}
      title={title ?? label}
      className={cn(
        "flex items-center gap-1 rounded-md border border-border px-1.5 py-1 text-[10px] hover:bg-accent",
        (disabled || busy) && "cursor-not-allowed opacity-50",
      )}
    >
      {busy ? <Loader2 size={10} className="animate-spin" /> : <Icon size={10} />}
      {label}
    </button>
  );
}
