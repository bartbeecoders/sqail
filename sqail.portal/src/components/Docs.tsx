import { useState } from "react";
import { ChevronDown, Rocket, Plug, Sparkles, Keyboard } from "lucide-react";

interface DocSection {
  id: string;
  title: string;
  icon: React.ReactNode;
  body: React.ReactNode;
}

const SECTIONS: DocSection[] = [
  {
    id: "getting-started",
    title: "Getting Started",
    icon: <Rocket size={18} />,
    body: (
      <div className="space-y-4 text-sm leading-relaxed text-text-muted">
        <p>
          Download sqail for your OS from the{" "}
          <a href="#download" className="text-brand-cyan hover:underline">
            Downloads
          </a>{" "}
          section and install it. sqail runs as a native desktop app on Windows,
          macOS, and Linux. No browser, no sign-up, no account.
        </p>
        <div className="overflow-hidden rounded-xl border border-border">
          <img
            src="/screenshots/editor.png"
            alt="sqail editor — write SQL, see results"
            className="h-auto w-full"
          />
        </div>
        <ol className="ml-5 list-decimal space-y-2">
          <li>Launch sqail. You'll land on an empty workspace.</li>
          <li>
            Open the left sidebar and click{" "}
            <span className="font-mono text-text-primary">+ New Connection</span>.
          </li>
          <li>
            Pick a driver, enter your credentials, and click{" "}
            <span className="font-mono text-text-primary">Test</span> before
            saving.
          </li>
          <li>
            Open a new query tab, write SQL, and press{" "}
            <span className="font-mono text-text-primary">F5</span> to run it.
          </li>
        </ol>
        <p>
          Everything is keyboard-driven. Press{" "}
          <span className="font-mono text-text-primary">Ctrl+K</span> to summon
          the AI command palette at any time.
        </p>
      </div>
    ),
  },
  {
    id: "connections",
    title: "Connections",
    icon: <Plug size={18} />,
    body: (
      <div className="space-y-4 text-sm leading-relaxed text-text-muted">
        <p>
          sqail speaks four database dialects out of the box. Credentials are
          stored locally in an encrypted SurrealDB embedded store — they never
          leave your machine.
        </p>
        <ul className="ml-5 list-disc space-y-1">
          <li>
            <span className="text-text-primary">PostgreSQL</span> — schemas,
            extensions, advanced types, notifications
          </li>
          <li>
            <span className="text-text-primary">MySQL / MariaDB</span> — dialect
            highlighting, utf8mb4, full DDL
          </li>
          <li>
            <span className="text-text-primary">SQLite</span> — local files,
            zero config
          </li>
          <li>
            <span className="text-text-primary">SQL Server</span> — Windows,
            Entra ID, and SQL authentication
          </li>
        </ul>
        <p>
          SSH tunnels are supported for Postgres and MySQL — point sqail at your
          bastion host once and sqail will manage the tunnel lifecycle. Secrets
          entered into tunnel fields are stored alongside the connection, also
          encrypted.
        </p>
        <p>
          Need to share a connection profile across machines without exposing
          credentials? Run the optional{" "}
          <span className="font-mono text-text-primary">sqail-dbservice</span>{" "}
          backend and point sqail at its HTTPS endpoint instead of the raw
          database.
        </p>
      </div>
    ),
  },
  {
    id: "ai-setup",
    title: "AI Setup",
    icon: <Sparkles size={18} />,
    body: (
      <div className="space-y-4 text-sm leading-relaxed text-text-muted">
        <p>
          sqail is bring-your-own-key. You pick the provider, you hold the
          credential, and sqail only calls out when you ask it to.
        </p>
        <p>
          Open{" "}
          <span className="font-mono text-text-primary">Settings → AI</span>{" "}
          (or press{" "}
          <span className="font-mono text-text-primary">Ctrl+Shift+A</span>),
          pick a provider, and paste your key:
        </p>
        <ul className="ml-5 list-disc space-y-1">
          <li>Claude (Anthropic)</li>
          <li>OpenAI</li>
          <li>Minimax</li>
          <li>Z.ai</li>
          <li>LM Studio (local)</li>
          <li>Claude Code CLI (local)</li>
          <li>Any OpenAI-compatible endpoint</li>
        </ul>
        <p>
          Once a provider is configured, press{" "}
          <span className="font-mono text-text-primary">Ctrl+K</span> to open
          the AI command palette. Ask in plain English — sqail injects the
          current schema, so "top 10 customers by revenue last quarter" resolves
          against your real table and column names, not a guess.
        </p>
        <p className="rounded-lg border border-brand-yellow/30 bg-brand-yellow/5 p-3 text-xs text-text-muted">
          <span className="font-semibold text-brand-yellow">
            Privacy note:
          </span>{" "}
          sqail sends your prompt, your schema (table and column names only,
          never data), and the selected SQL to your chosen provider. It never
          sends row contents or credentials. Choose a local provider (LM Studio,
          Claude Code CLI) if you need zero network egress.
        </p>
      </div>
    ),
  },
  {
    id: "shortcuts",
    title: "Keyboard Shortcuts",
    icon: <Keyboard size={18} />,
    body: (
      <div className="space-y-4 text-sm leading-relaxed text-text-muted">
        <p>
          Defaults below. Every shortcut is rebindable in{" "}
          <span className="font-mono text-text-primary">
            Settings → Shortcuts
          </span>
          . On macOS,{" "}
          <span className="font-mono text-text-primary">Ctrl</span> maps to{" "}
          <span className="font-mono text-text-primary">Cmd</span>.
        </p>
        <div className="overflow-x-auto rounded-lg border border-border">
          <table className="w-full text-sm">
            <thead className="bg-bg-card text-text-primary">
              <tr>
                <th className="px-4 py-2 text-left font-semibold">Category</th>
                <th className="px-4 py-2 text-left font-semibold">Action</th>
                <th className="px-4 py-2 text-left font-semibold">Default</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border text-text-muted">
              <tr>
                <td className="px-4 py-2">Editor</td>
                <td className="px-4 py-2">Run Query</td>
                <td className="px-4 py-2 font-mono text-text-primary">F5</td>
              </tr>
              <tr>
                <td className="px-4 py-2">Editor</td>
                <td className="px-4 py-2">Format Query</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+Shift+F
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">Editor</td>
                <td className="px-4 py-2">New Tab</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+N
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">Editor</td>
                <td className="px-4 py-2">Close Tab</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+W
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">File</td>
                <td className="px-4 py-2">Save Query</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+S
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">File</td>
                <td className="px-4 py-2">Save Query As</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+Shift+S
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">File</td>
                <td className="px-4 py-2">Open Query</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+O
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">Connections</td>
                <td className="px-4 py-2">New Connection</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+Shift+N
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">AI</td>
                <td className="px-4 py-2">AI Command Palette</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+K
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">AI</td>
                <td className="px-4 py-2">Toggle AI Settings</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+Shift+A
                </td>
              </tr>
              <tr>
                <td className="px-4 py-2">App</td>
                <td className="px-4 py-2">Open Settings</td>
                <td className="px-4 py-2 font-mono text-text-primary">
                  Ctrl+,
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
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
            The short version. Everything you need to get from install to first
            AI-assisted query, plus the canonical shortcut list.
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
