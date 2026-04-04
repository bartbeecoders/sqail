import { Download, ExternalLink } from "lucide-react";
import { VERSION, DOWNLOADS, GITHUB_URL, detectPlatform, getDownloadUrl } from "../lib/constants";

export default function Hero() {
  const platform = detectPlatform();
  const primaryDownload = DOWNLOADS.find((d) => d.platform === platform) ?? DOWNLOADS[2];

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
            <span className="text-xs text-text-muted">v{VERSION} — Now available</span>
          </div>

          <h1 className="mb-6 text-4xl leading-tight font-bold tracking-tight text-text-primary sm:text-5xl lg:text-6xl">
            The SQL editor that{" "}
            <span className="bg-gradient-to-r from-brand-cyan to-brand-yellow bg-clip-text text-transparent">
              thinks with you
            </span>
          </h1>

          <p className="mb-8 max-w-lg text-lg leading-relaxed text-text-muted">
            A lightweight, cross-platform desktop SQL editor with built-in AI.
            Connect to PostgreSQL, MySQL, SQLite, or SQL Server — and let AI
            help you write, explain, and optimize your queries.
          </p>

          <div className="flex flex-wrap gap-4">
            <a
              href={getDownloadUrl(primaryDownload.fileName)}
              className="inline-flex items-center gap-2 rounded-lg bg-brand-cyan px-6 py-3 font-semibold text-bg-primary transition-colors hover:bg-brand-cyan/85"
            >
              <Download size={18} />
              Download for {primaryDownload.label}
            </a>
            <a
              href={GITHUB_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 rounded-lg border border-border px-6 py-3 font-semibold text-text-primary transition-colors hover:border-text-muted hover:bg-bg-section"
            >
              <ExternalLink size={18} />
              View on GitHub
            </a>
          </div>
        </div>

        {/* Hero image */}
        <div className="flex items-center justify-center">
          <div className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-border shadow-2xl shadow-brand-cyan/5">
            <img
              src={new URL("../assets/hero.jpg", import.meta.url).href}
              alt="SQaiL — database tables and queries visualized"
              className="h-auto w-full"
            />
          </div>
        </div>
      </div>
    </section>
  );
}
