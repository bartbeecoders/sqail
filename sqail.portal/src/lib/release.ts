import { useEffect, useState } from "react";

/**
 * The current release, as written to /releases/current.json by the release
 * workflow next to the files it uploads. Read at runtime so the portal never
 * needs a rebuild to show a new version.
 */
export interface Release {
  version: string;
  date: string;
  files: string[];
}

export interface ReleaseFiles {
  linuxTar?: string;
  windowsMsi?: string;
  windowsZip?: string;
}

export function releaseFiles(release: Release | null): ReleaseFiles {
  const find = (suffix: string) => release?.files.find((f) => f.endsWith(suffix));
  return {
    linuxTar: find("-linux-x86_64.tar.gz"),
    windowsMsi: find("-windows-x64.msi"),
    windowsZip: find("-windows-x64.zip"),
  };
}

let cached: Promise<Release | null> | null = null;

function load(): Promise<Release | null> {
  cached ??= fetch("/releases/current.json", { cache: "no-cache" })
    .then((r) => (r.ok ? (r.json() as Promise<Release>) : null))
    .then((r) => (r && typeof r.version === "string" && Array.isArray(r.files) ? r : null))
    .catch(() => null);
  return cached;
}

/** `undefined` while loading, `null` when no release is published. */
export function useRelease(): Release | null | undefined {
  const [release, setRelease] = useState<Release | null | undefined>(undefined);
  useEffect(() => {
    let live = true;
    load().then((r) => live && setRelease(r));
    return () => {
      live = false;
    };
  }, []);
  return release;
}
