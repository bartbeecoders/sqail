import { Download, Info, Monitor, Server, Terminal } from "lucide-react";
import {
  VERSION,
  BUILD_NUMBER,
  DOWNLOADS,
  DBSERVICE_DOWNLOADS,
  detectPlatform,
  getDownloadUrl,
} from "../lib/constants";

const ICON_MAP: Record<string, React.ReactNode> = {
  Monitor: <Monitor size={20} />,
  Apple: (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-5 w-5">
      <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.8-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z" />
    </svg>
  ),
  Terminal: <Terminal size={20} />,
};

export default function Downloads() {
  const platform = detectPlatform();

  return (
    <section id="download" className="bg-bg-section py-24">
      <div className="mx-auto max-w-4xl px-6 text-center">
        <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
          Download SQaiL
        </h2>
        <p className="mb-12 text-text-muted">
          Free, open source, and ready to go. Pick your platform.
        </p>

        <div className="grid gap-4 sm:grid-cols-3">
          {DOWNLOADS.map((dl) => {
            const isCurrent = dl.platform === platform;
            return (
              <a
                key={dl.platform}
                href={getDownloadUrl(dl.fileName)}
                className={`group flex flex-col items-center gap-3 rounded-xl border p-8 transition-colors ${
                  isCurrent
                    ? "border-brand-cyan bg-brand-cyan/5 hover:bg-brand-cyan/10"
                    : "border-border bg-bg-primary hover:border-text-dim hover:bg-bg-card"
                }`}
              >
                <div
                  className={`flex h-12 w-12 items-center justify-center rounded-full ${
                    isCurrent
                      ? "bg-brand-cyan/15 text-brand-cyan"
                      : "bg-bg-section text-text-muted group-hover:text-text-primary"
                  }`}
                >
                  {ICON_MAP[dl.icon]}
                </div>
                <span className="text-lg font-semibold text-text-primary">
                  {dl.label}
                </span>
                <span className="text-xs text-text-dim">{dl.fileName}</span>
                <span className="inline-flex items-center gap-1 text-sm text-brand-cyan">
                  <Download size={14} />
                  Download
                </span>
              </a>
            );
          })}
        </div>

        {/* macOS install note — unsigned app workaround */}
        <div
          className={`mt-8 rounded-xl border p-5 text-left transition-colors ${
            platform === "macos"
              ? "border-brand-yellow/40 bg-brand-yellow/5"
              : "border-border bg-bg-primary"
          }`}
        >
          <div className="mb-2 flex items-center gap-2 text-sm font-semibold text-text-primary">
            <Info size={16} className="text-brand-yellow" />
            macOS: "sqail is damaged and cannot be opened"?
          </div>
          <p className="mb-3 text-sm text-text-muted">
            sqail is not yet signed with an Apple Developer ID, so macOS
            Gatekeeper flags it after download. The app is fine — you just need
            to remove the quarantine attribute once. Drag sqail to{" "}
            <span className="font-mono text-text-primary">/Applications</span>,
            then run:
          </p>
          <pre className="overflow-x-auto rounded-lg border border-border bg-bg-primary px-4 py-3 text-xs text-brand-cyan">
            <code>xattr -cr /Applications/sqail.app</code>
          </pre>
          <p className="mt-3 text-xs text-text-dim">
            You only need to do this once per install. Proper signing +
            notarization is on the roadmap.
          </p>
        </div>

        {/* DbService — optional backend service */}
        <div className="mt-16">
          <h3 className="mb-2 text-xl font-semibold text-text-primary">
            DbService (optional backend)
          </h3>
          <p className="mb-6 text-sm text-text-muted">
            A standalone HTTP service that proxies queries to remote databases.
            Install on a server to share connections without exposing credentials to clients.
          </p>
          <div className="grid gap-4 sm:grid-cols-3">
            {DBSERVICE_DOWNLOADS.map((dl) => (
              <a
                key={dl.platform}
                href={getDownloadUrl(dl.fileName)}
                className="group flex flex-col items-center gap-3 rounded-xl border border-border bg-bg-primary p-6 transition-colors hover:border-text-dim hover:bg-bg-card"
              >
                <div className="flex h-12 w-12 items-center justify-center rounded-full bg-bg-section text-text-muted group-hover:text-text-primary">
                  <Server size={20} />
                </div>
                <span className="text-lg font-semibold text-text-primary">
                  {dl.label}
                </span>
                <span className="break-all text-xs text-text-dim">{dl.fileName}</span>
                <span className="inline-flex items-center gap-1 text-sm text-brand-cyan">
                  <Download size={14} />
                  Download
                </span>
              </a>
            ))}
          </div>
        </div>

        <div className="mt-8 flex items-center justify-center gap-6 text-sm text-text-dim">
          <span>v{VERSION}</span>
          <span className="text-border">|</span>
          <span>Build {BUILD_NUMBER}</span>
          <span className="text-border">|</span>
          <a
            href="#changelog"
            className="inline-flex items-center gap-1 transition-colors hover:text-brand-cyan"
          >
            Changelog
          </a>
        </div>
      </div>
    </section>
  );
}
