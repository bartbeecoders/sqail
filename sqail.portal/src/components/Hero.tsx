import { Download, ExternalLink } from "lucide-react";
import { CODEBERG_URL, detectPlatform, getDownloadUrl } from "../lib/constants";
import { releaseFiles, useRelease } from "../lib/release";

export default function Hero() {
  const platform = detectPlatform();
  const release = useRelease();
  const { windowsMsi } = releaseFiles(release ?? null);

  // Windows: the MSI directly. Linux: pick a format in #download. macOS: no build.
  const ctaHref = platform === "windows" && windowsMsi ? getDownloadUrl(windowsMsi) : "#download";
  const ctaLabel =
    platform === "windows" ? "Download for Windows" : platform === "linux" ? "Download for Linux" : "Download";

  return (
    <section className="relative flex min-h-screen items-center overflow-hidden pt-16">
      {/* Background gradient */}
      <div className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_top_right,_rgba(56,189,248,0.08)_0%,_transparent_60%)]" />
      <div className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_left,_rgba(251,191,36,0.06)_0%,_transparent_60%)]" />

      <div className="relative mx-auto grid max-w-6xl gap-12 px-6 lg:grid-cols-2 lg:gap-16">
        {/* Text */}
        <div className="flex flex-col justify-center">
          <div className="mb-6 inline-flex w-fit items-center gap-2 rounded-full border border-border bg-bg-section px-3 py-1">
            <span className="h-2 w-2 rounded-full bg-brand-cyan" />
            <span className="text-xs text-text-muted">
              {release ? `v${release.version} — rewritten in Rust` : "Rewritten in Rust"}
            </span>
          </div>

          <h1 className="mb-6 text-4xl leading-tight font-bold tracking-tight text-text-primary sm:text-5xl lg:text-6xl">
            The SQL editor that{" "}
            <span className="bg-gradient-to-r from-brand-cyan to-brand-yellow bg-clip-text text-transparent">
              keeps up with you
            </span>
          </h1>

          <p className="mb-6 max-w-lg text-lg leading-relaxed text-text-muted">
            A native desktop SQL editor for PostgreSQL, SQL Server and SQLite. It
            streams millions of rows without blinking, and it keeps database
            credentials off your machine: every query goes through sqail-service,
            a small HTTPS gateway you run locally or share with your team.
          </p>

          <div className="mb-8 flex flex-wrap gap-2">
            <span className="rounded-full border border-brand-cyan/30 bg-brand-cyan/5 px-3 py-1 text-xs font-medium text-brand-cyan">
              Linux &amp; Windows
            </span>
            <span className="rounded-full border border-brand-yellow/30 bg-brand-yellow/5 px-3 py-1 text-xs font-medium text-brand-yellow">
              Admin page for the service
            </span>
          </div>

          <div className="flex flex-wrap gap-4">
            <a
              href={ctaHref}
              className="inline-flex items-center gap-2 rounded-lg bg-brand-cyan px-6 py-3 font-semibold text-bg-primary transition-colors hover:bg-brand-cyan/85"
            >
              <Download size={18} />
              {ctaLabel}
            </a>
            <a
              href={CODEBERG_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 rounded-lg border border-border px-6 py-3 font-semibold text-text-primary transition-colors hover:border-text-muted hover:bg-bg-section"
            >
              <ExternalLink size={18} />
              Source code
            </a>
          </div>
        </div>

        {/* Hero image */}
        <div className="flex items-center justify-center">
          <div className="relative w-full overflow-hidden rounded-2xl border border-border shadow-2xl shadow-brand-cyan/5">
            <img
              src="/screenshots/million-rows.png"
              alt="sqail showing a million-row PostgreSQL result"
              className="h-auto w-full"
            />
          </div>
        </div>
      </div>
    </section>
  );
}
