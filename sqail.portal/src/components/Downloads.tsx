import type { ReactNode } from "react";
import { Archive, Download, History, Info, Monitor, Terminal } from "lucide-react";
import {
  LEGACY_URL,
  LEGACY_VERSION,
  RELEASES_URL,
  detectPlatform,
  docUrl,
  getDownloadUrl,
} from "../lib/constants";
import { releaseFiles, useRelease } from "../lib/release";

interface CardProps {
  icon: ReactNode;
  title: string;
  subtitle: string;
  current: boolean;
  children: ReactNode;
}

function PlatformCard({ icon, title, subtitle, current, children }: CardProps) {
  return (
    <div
      className={`rounded-xl border p-6 text-left transition-colors ${
        current ? "border-brand-cyan bg-brand-cyan/5" : "border-border bg-bg-primary"
      }`}
    >
      <div className="mb-4 flex items-center gap-3">
        <div
          className={`flex h-10 w-10 items-center justify-center rounded-full ${
            current ? "bg-brand-cyan/15 text-brand-cyan" : "bg-bg-section text-text-muted"
          }`}
        >
          {icon}
        </div>
        <div>
          <h3 className="text-lg font-semibold text-text-primary">{title}</h3>
          <p className="text-xs text-text-dim">{subtitle}</p>
        </div>
      </div>
      <div className="space-y-3">{children}</div>
    </div>
  );
}

function FileLink({ file, label, description }: { file: string; label: string; description: string }) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-border px-4 py-3 transition-colors hover:border-brand-cyan/60 hover:bg-bg-card">
      <a href={getDownloadUrl(file)} className="flex min-w-0 flex-1 items-center gap-3">
        <div className="min-w-0 flex-1">
          <span className="block text-sm font-semibold text-text-primary">{label}</span>
          <span className="block truncate text-xs text-text-dim">{description}</span>
        </div>
        <Download size={14} className="shrink-0 text-brand-cyan" />
      </a>
      <a
        href={getDownloadUrl(`${file}.sha256`)}
        className="shrink-0 text-xs text-text-dim hover:text-brand-cyan"
        title="SHA-256 checksum"
      >
        sha256
      </a>
    </div>
  );
}

export default function Downloads() {
  const platform = detectPlatform();
  const release = useRelease();
  const files = releaseFiles(release ?? null);
  const missing = release === null;

  return (
    <section id="download" className="bg-bg-section py-24">
      <div className="mx-auto max-w-4xl px-6 text-center">
        <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">Download sqail</h2>
        <p className="mb-12 text-text-muted">
          Free and open source. Each download contains the editor and sqail-service.
        </p>

        {missing && (
          <p className="mb-8 rounded-xl border border-border bg-bg-primary p-5 text-sm text-text-muted">
            The downloads are on the{" "}
            <a href={RELEASES_URL} className="text-brand-cyan hover:underline">
              GitHub releases page
            </a>
            .
          </p>
        )}

        <div className="grid gap-6 sm:grid-cols-2">
          <PlatformCard
            icon={<Monitor size={20} />}
            title="Windows"
            subtitle="Windows 10 / 11, x64"
            current={platform === "windows"}
          >
            {files.windowsMsi && (
              <FileLink file={files.windowsMsi} label="Installer (.msi)" description="Installs to Program Files" />
            )}
            {files.windowsZip && (
              <FileLink file={files.windowsZip} label="Portable (.zip)" description="Unzip anywhere and run sqail.exe" />
            )}
            <p className="text-xs text-text-dim">
              Setting up a shared service with SQL Server?{" "}
              <a href={docUrl("windows-setup.md")} className="text-brand-cyan hover:underline">
                Step-by-step guide
              </a>
            </p>
          </PlatformCard>

          <PlatformCard
            icon={<Terminal size={20} />}
            title="Linux"
            subtitle="x86_64, Wayland or X11"
            current={platform === "linux"}
          >
            {files.linuxTar && (
              <FileLink file={files.linuxTar} label="Tarball (.tar.gz)" description="Unpack and run ./install.sh" />
            )}
            <div className="rounded-lg border border-border px-4 py-3">
              <span className="block text-sm font-semibold text-text-primary">Arch / Omarchy</span>
              <span className="block text-xs text-text-dim">
                Build the package: <code className="text-brand-cyan">makepkg -si</code> in{" "}
                <code className="text-brand-cyan">packaging/arch/</code>
              </span>
            </div>
          </PlatformCard>
        </div>

        {platform === "macos" && (
          <div className="mt-8 rounded-xl border border-brand-yellow/40 bg-brand-yellow/5 p-5 text-left">
            <div className="mb-2 flex items-center gap-2 text-sm font-semibold text-text-primary">
              <Info size={16} className="text-brand-yellow" />
              No macOS build yet
            </div>
            <p className="text-sm text-text-muted">
              sqail currently ships for Linux and Windows. You can run sqail-service on any server and connect to
              it from a Linux or Windows machine.
            </p>
          </div>
        )}

        {/* Legacy Tauri app */}
        <div className="mt-8 flex items-start gap-3 rounded-xl border border-border bg-bg-primary p-5 text-left">
          <History size={18} className="mt-0.5 shrink-0 text-text-dim" />
          <p className="text-sm text-text-muted">
            Looking for the previous sqail (0.x, built with Tauri, with AI features, MySQL and macOS)? It is no
            longer developed, but{" "}
            <a href={LEGACY_URL} className="text-brand-cyan hover:underline">
              v{LEGACY_VERSION}
            </a>{" "}
            is still available.
          </p>
        </div>

        <div className="mt-8 flex items-center justify-center gap-6 text-sm text-text-dim">
          {release && (
            <>
              <span>v{release.version}</span>
              <span className="text-border">|</span>
              <span>{release.date}</span>
              <span className="text-border">|</span>
            </>
          )}
          <a href={RELEASES_URL} className="inline-flex items-center gap-1 transition-colors hover:text-brand-cyan">
            <Archive size={14} />
            All releases
          </a>
          <span className="text-border">|</span>
          <a href="#changelog" className="transition-colors hover:text-brand-cyan">
            Changelog
          </a>
        </div>
      </div>
    </section>
  );
}
