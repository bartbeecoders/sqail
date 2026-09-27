import { useState } from "react";
import { BookOpen, ChevronDown, Keyboard, Rocket, Server } from "lucide-react";
import { docUrl } from "../lib/constants";

interface DocSection {
  id: string;
  title: string;
  icon: React.ReactNode;
  body: React.ReactNode;
}

const kbd = "font-mono text-text-primary";

const SHORTCUTS: [string, string][] = [
  ["Ctrl+Enter", "Run the statement at the cursor, or the selection"],
  ["F5 / Ctrl+Shift+Enter", "Run the whole script"],
  ["Esc", "Cancel the running query"],
  ["Ctrl+E", "Show the plan of the statement at the cursor"],
  ["Ctrl+Space", "Complete (also opens by itself while typing)"],
  ["Ctrl+Shift+F", "Format the selection or the whole tab"],
  ["Ctrl+Shift+P", "Command palette"],
  ["Ctrl+P", "Quick open: a table or snippet"],
  ["Ctrl+T / Ctrl+W", "New / close tab"],
  ["Ctrl+O / Ctrl+S / Ctrl+Shift+S", "Open / save / save as"],
  ["Ctrl+F", "Find and replace"],
  ["F2", "Edit the selected cell (in edit mode)"],
];

const DOCS: [string, string, string][] = [
  ["User guide", "user-guide.md", "Everything the editor does, settings and files"],
  ["Windows + SQL Server setup", "windows-setup.md", "A shared service on Windows, step by step"],
  ["Operations", "operations.md", "Certificates, tokens, backups, running as a service"],
  ["Security", "security.md", "Threat model and what protects against what"],
  ["REST API", "api.md", "Use sqail-service from scripts"],
];

const SECTIONS: DocSection[] = [
  {
    id: "getting-started",
    title: "Getting started",
    icon: <Rocket size={18} />,
    body: (
      <div className="space-y-4 text-sm leading-relaxed text-text-muted">
        <ol className="ml-5 list-decimal space-y-2">
          <li>
            Install sqail from the{" "}
            <a href="#download" className="text-brand-cyan hover:underline">
              downloads
            </a>
            : the MSI on Windows, <span className={kbd}>./install.sh</span> from the tarball on Linux.
          </li>
          <li>
            Start sqail and choose <span className={kbd}>Use the local service</span>. sqail starts sqail-service
            next to it, creates a token for you and pins its certificate. There is nothing else to set up.
          </li>
          <li>
            Click <span className={kbd}>+ Add</span> next to Connections and enter a PostgreSQL, SQL Server or
            SQLite connection. The password goes to the service, encrypted, and never comes back.
          </li>
          <li>
            Write SQL and press <span className={kbd}>Ctrl+Enter</span> to run the statement at the cursor.
          </li>
        </ol>
      </div>
    ),
  },
  {
    id: "shared-service",
    title: "Connecting to a shared service",
    icon: <Server size={18} />,
    body: (
      <div className="space-y-4 text-sm leading-relaxed text-text-muted">
        <p>
          On a server, install sqail-service (<span className={kbd}>setup\Install-SqailService.cmd</span> on
          Windows, <span className={kbd}>install.sh</span> on Linux). The installer prints a sign-in link for the
          admin page at <span className={kbd}>https://&lt;host&gt;:7443/admin/</span>, where you set the network,
          certificate, connections and tokens.
        </p>
        <p>
          In sqail, choose <span className={kbd}>Service → Connect to a service…</span> and enter the URL and the
          token you were given. With a self-signed certificate, sqail shows its fingerprint. Accept it only if it
          matches the one on the admin page.
        </p>
      </div>
    ),
  },
  {
    id: "shortcuts",
    title: "Keyboard shortcuts",
    icon: <Keyboard size={18} />,
    body: (
      <div className="space-y-4 text-sm leading-relaxed text-text-muted">
        <p>
          The defaults. Rebind any of them in <span className={kbd}>keybindings.toml</span> in the config
          folder, for example <span className={kbd}>"query.run" = "Ctrl+R"</span>.
        </p>
        <div className="overflow-x-auto rounded-lg border border-border">
          <table className="w-full text-sm">
            <thead className="bg-bg-card text-text-primary">
              <tr>
                <th className="px-4 py-2 text-left font-semibold">Keys</th>
                <th className="px-4 py-2 text-left font-semibold">Action</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border text-text-muted">
              {SHORTCUTS.map(([keys, action]) => (
                <tr key={keys}>
                  <td className="px-4 py-2 font-mono whitespace-nowrap text-text-primary">{keys}</td>
                  <td className="px-4 py-2">{action}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    ),
  },
  {
    id: "full-docs",
    title: "Full documentation",
    icon: <BookOpen size={18} />,
    body: (
      <ul className="space-y-3 text-sm">
        {DOCS.map(([title, file, description]) => (
          <li key={file}>
            <a href={docUrl(file)} className="font-semibold text-brand-cyan hover:underline">
              {title}
            </a>
            <span className="text-text-muted"> — {description}</span>
          </li>
        ))}
      </ul>
    ),
  },
];

export default function Docs() {
  const [open, setOpen] = useState<string | null>("getting-started");

  return (
    <section id="docs" className="py-24">
      <div className="mx-auto max-w-4xl px-6">
        <div className="mb-12 text-center">
          <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
            Docs
          </h2>
          <p className="mx-auto max-w-2xl text-text-muted">
            The short version: from install to your first query, plus the
            shortcut list. The full guides live in the repository.
          </p>
        </div>

        <div className="space-y-3">
          {SECTIONS.map((section) => {
            const isOpen = open === section.id;
            return (
              <div
                key={section.id}
                className="overflow-hidden rounded-xl border border-border bg-bg-section"
              >
                <button
                  onClick={() => setOpen(isOpen ? null : section.id)}
                  className="flex w-full items-center justify-between px-6 py-4 text-left transition-colors hover:bg-bg-card"
                  aria-expanded={isOpen}
                  aria-controls={`doc-panel-${section.id}`}
                >
                  <div className="flex items-center gap-3">
                    <span className="text-brand-cyan">{section.icon}</span>
                    <span className="font-semibold text-text-primary">
                      {section.title}
                    </span>
                  </div>
                  <ChevronDown
                    size={18}
                    className={`text-text-muted transition-transform ${
                      isOpen ? "rotate-180" : ""
                    }`}
                  />
                </button>
                {isOpen && (
                  <div
                    id={`doc-panel-${section.id}`}
                    className="border-t border-border px-6 py-5"
                  >
                    {section.body}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
