/**
 * Git integration types (Phase 1). Backend-facing — fields are camelCase as
 * emitted by the Rust `serde(rename_all = "camelCase")` attrs on each struct.
 */

export interface GitCredentials {
  username?: string;
  password?: string;
}

export interface GitSignature {
  name: string;
  email: string;
}

export interface RepoInfo {
  path: string;
  headBranch: string | null;
  headCommit: string | null;
  isEmpty: boolean;
  remotes: string[];
}

export type GitFileKind =
  | "new"
  | "modified"
  | "deleted"
  | "renamed"
  | "typechange"
  | "conflicted";

export interface GitFileChange {
  path: string;
  kind: GitFileKind;
  staged: boolean;
  hasUnstaged: boolean;
}

export interface GitStatusResponse {
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  files: GitFileChange[];
}

export interface GitFileDiff {
  path: string;
  oldContent: string;
  newContent: string;
  patch: string;
}

export interface GitRemote {
  name: string;
  url: string;
}

export interface PullResult {
  kind: "up-to-date" | "fast-forward" | "merge-required" | "no-upstream";
  message: string;
  changedFiles: string[];
}

export interface SnapshotSummary {
  filesWritten: number;
  written: string[];
  removed: string[];
}

/**
 * Git configuration attached to a sqail Project. Stored inside the `.sqail`
 * file (and in localStorage for the active project) — never inside the git
 * repo itself.
 */
export interface ProjectGitConfig {
  /** Local filesystem path to the git working tree. */
  repoPath: string;
  /** Which sqail connection this repo snapshots. */
  connectionId?: string;
  /** Last remote the user fetched/pulled/pushed with. */
  defaultRemote?: string;
  /** Whether to include stored routines in snapshots. */
  includeRoutines?: boolean;
  /** ISO timestamp of last snapshot. */
  lastSnapshotAt?: string;
}
