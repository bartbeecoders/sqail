import { ClipboardList, KeyRound, Lock, MonitorCog, Network, Server } from "lucide-react";
import { docUrl } from "../lib/constants";

const POINTS = [
  {
    icon: <Lock size={20} />,
    title: "Credentials stay on the service",
    description:
      "Database passwords are encrypted with AES-256-GCM and never sent back. The editor only ever holds a token.",
  },
  {
    icon: <KeyRound size={20} />,
    title: "Scoped tokens, pinned certificates",
    description:
      "Tokens are read, query or admin. HTTPS only (TLS 1.3), with trust on first use, your company CA, or mutual TLS.",
  },
  {
    icon: <MonitorCog size={20} />,
    title: "Admin page built in",
    description:
      "Connections, tokens, certificates, SQLite folders, limits, backups and settings, all managed in the browser.",
  },
  {
    icon: <ClipboardList size={20} />,
    title: "Audit log and limits",
    description:
      "The audit log records who ran what, where and for how long, never row values. Row caps, timeouts and per-token rate limits are enforced on the server.",
  },
];

export default function ServiceSection() {
  return (
    <section id="service" className="bg-bg-section py-24">
      <div className="mx-auto max-w-6xl px-6">
        <div className="grid gap-16 lg:grid-cols-2">
          <div>
            <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
              One gateway between you and your databases
            </h2>
            <p className="mb-6 text-text-muted">
              sqail never connects to a database itself. <strong className="text-text-primary">sqail-service</strong>{" "}
              holds the connection profiles and streams results to the editor over HTTPS. Run it on your own
              machine (sqail sets it up with one click) or on a server, so that a whole team shares the same
              connections without ever seeing a password.
            </p>

            {/* Diagram */}
            <div className="mb-8 flex flex-wrap items-center gap-3 text-sm">
              <span className="inline-flex items-center gap-2 rounded-lg border border-border bg-bg-primary px-3 py-2 text-text-primary">
                <Network size={16} className="text-brand-cyan" /> sqail
              </span>
              <span className="text-text-dim">— HTTPS →</span>
              <span className="inline-flex items-center gap-2 rounded-lg border border-brand-yellow/40 bg-bg-primary px-3 py-2 text-text-primary">
                <Server size={16} className="text-brand-yellow" /> sqail-service
              </span>
              <span className="text-text-dim">→</span>
              <span className="rounded-lg border border-border bg-bg-primary px-3 py-2 text-text-muted">
                PostgreSQL · SQL Server · SQLite
              </span>
            </div>

            <p className="text-sm text-text-muted">
              Runs as a systemd user unit on Linux or a Windows service. The REST API is documented with
              OpenAPI, so scripts can use it as well. Read the{" "}
              <a href={docUrl("operations.md")} className="text-brand-cyan hover:underline">
                operations guide
              </a>{" "}
              and the{" "}
              <a href={docUrl("security.md")} className="text-brand-cyan hover:underline">
                security notes
              </a>
              .
            </p>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            {POINTS.map((p) => (
              <div key={p.title} className="rounded-xl border border-border bg-bg-primary p-5">
                <div className="mb-3 inline-flex rounded-lg bg-brand-yellow/10 p-2 text-brand-yellow">{p.icon}</div>
                <h3 className="mb-1 font-semibold text-text-primary">{p.title}</h3>
                <p className="text-sm leading-relaxed text-text-muted">{p.description}</p>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
