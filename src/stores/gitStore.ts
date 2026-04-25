import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  GitCredentials,
  GitFileDiff,
  GitRemote,
  GitStatusResponse,
  PullResult,
  RepoInfo,
  SnapshotSummary,
} from "../types/git";

interface GitState {
  repoInfo: RepoInfo | null;
  status: GitStatusResponse | null;
  branches: string[];
  remotes: GitRemote[];
  loading: boolean;
  error: string | null;

  openRepo: (path: string) => Promise<RepoInfo>;
  initRepo: (path: string) => Promise<RepoInfo>;
  cloneRepo: (url: string, dest: string, creds?: GitCredentials) => Promise<RepoInfo>;

  refresh: (path: string) => Promise<void>;

  stage: (path: string, paths: string[]) => Promise<void>;
  unstage: (path: string, paths: string[]) => Promise<void>;
  stageAll: (path: string) => Promise<void>;
  commit: (
    path: string,
    message: string,
    signature?: { name: string; email: string },
  ) => Promise<string>;

  setRemote: (path: string, name: string, url: string) => Promise<void>;
  removeRemote: (path: string, name: string) => Promise<void>;

  fetch: (path: string, remote: string, creds?: GitCredentials) => Promise<void>;
  pull: (path: string, remote: string, creds?: GitCredentials) => Promise<PullResult>;
  push: (path: string, remote: string, branch: string, creds?: GitCredentials) => Promise<void>;

  fileDiff: (path: string, file: string) => Promise<GitFileDiff>;
  changedFiles: (path: string) => Promise<GitFileDiff[]>;

  snapshotSchema: (
    path: string,
    connectionId: string,
    includeRoutines?: boolean,
  ) => Promise<SnapshotSummary>;

  reset: () => void;
}

const INITIAL: Pick<
  GitState,
  "repoInfo" | "status" | "branches" | "remotes" | "loading" | "error"
> = {
  repoInfo: null,
  status: null,
  branches: [],
  remotes: [],
  loading: false,
  error: null,
};

export const useGitStore = create<GitState>((set, get) => ({
  ...INITIAL,

  openRepo: async (path) => {
    set({ loading: true, error: null });
    try {
      const info = await invoke<RepoInfo>("git_open_repo", { repoPath: path });
      set({ repoInfo: info });
      await get().refresh(path);
      return info;
    } catch (e) {
      set({ error: String(e) });
      throw e;
    } finally {
      set({ loading: false });
    }
  },

  initRepo: async (path) => {
    set({ loading: true, error: null });
    try {
      const info = await invoke<RepoInfo>("git_init_repo", { repoPath: path });
      set({ repoInfo: info });
      await get().refresh(path);
      return info;
    } catch (e) {
      set({ error: String(e) });
      throw e;
    } finally {
      set({ loading: false });
    }
  },

  cloneRepo: async (url, dest, creds) => {
    set({ loading: true, error: null });
    try {
      const info = await invoke<RepoInfo>("git_clone_repo", {
        url,
        dest,
        credentials: creds,
      });
      set({ repoInfo: info });
      await get().refresh(dest);
      return info;
    } catch (e) {
      set({ error: String(e) });
      throw e;
    } finally {
      set({ loading: false });
    }
  },

  refresh: async (path) => {
    try {
      const [status, branches, remotes] = await Promise.all([
        invoke<GitStatusResponse>("git_status", { repoPath: path }),
        invoke<string[]>("git_list_branches", { repoPath: path }),
        invoke<GitRemote[]>("git_list_remotes", { repoPath: path }),
      ]);
      set({ status, branches, remotes, error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  stage: async (path, paths) => {
    await invoke("git_stage", { repoPath: path, paths });
    await get().refresh(path);
  },

  unstage: async (path, paths) => {
    await invoke("git_unstage", { repoPath: path, paths });
    await get().refresh(path);
  },

  stageAll: async (path) => {
    await invoke("git_stage_all", { repoPath: path });
    await get().refresh(path);
  },

  commit: async (path, message, signature) => {
    const oid = await invoke<string>("git_commit", {
      repoPath: path,
      message,
      signature,
    });
    await get().refresh(path);
    return oid;
  },

  setRemote: async (path, name, url) => {
    await invoke("git_set_remote", { repoPath: path, name, url });
    await get().refresh(path);
  },

  removeRemote: async (path, name) => {
    await invoke("git_remove_remote", { repoPath: path, name });
    await get().refresh(path);
  },

  fetch: async (path, remote, creds) => {
    await invoke("git_fetch", { repoPath: path, remote, credentials: creds });
    await get().refresh(path);
  },

  pull: async (path, remote, creds) => {
    const result = await invoke<PullResult>("git_pull", {
      repoPath: path,
      remote,
      credentials: creds,
    });
    await get().refresh(path);
    return result;
  },

  push: async (path, remote, branch, creds) => {
    await invoke("git_push", {
      repoPath: path,
      remote,
      branch,
      credentials: creds,
    });
    await get().refresh(path);
  },

  fileDiff: (path, file) =>
    invoke<GitFileDiff>("git_file_diff", { repoPath: path, file }),

  changedFiles: (path) =>
    invoke<GitFileDiff[]>("git_changed_files", { repoPath: path }),

  snapshotSchema: async (path, connectionId, includeRoutines) => {
    const summary = await invoke<SnapshotSummary>("git_snapshot_schema", {
      repoPath: path,
      connectionId,
      includeRoutines,
    });
    await get().refresh(path);
    return summary;
  },

  reset: () => set({ ...INITIAL }),
}));
