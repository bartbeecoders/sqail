import { GITHUB_URL } from "../lib/constants";

export default function Footer() {
  return (
    <footer className="border-t border-border py-12">
      <div className="mx-auto max-w-6xl px-6">
        <div className="grid gap-8 sm:grid-cols-3">
          {/* Brand */}
          <div>
            <div className="mb-3 flex items-center gap-2">
              <img src="/icon.png" alt="sqail" className="h-6 w-6" />
              <span className="font-bold text-text-primary">sqail</span>
            </div>
            <p className="text-sm text-text-muted">
              A fast, native SQL editor for PostgreSQL, SQL Server and SQLite,
              with a secure gateway for your connections.
            </p>
          </div>

          {/* Links */}
          <div>
            <h4 className="mb-3 text-sm font-semibold text-text-primary">
              Links
            </h4>
            <ul className="space-y-2 text-sm text-text-muted">
              <li>
                <a
                  href={GITHUB_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="transition-colors hover:text-brand-cyan"
                >
                  Source Code
                </a>
              </li>
              <li>
                <a
                  href="#changelog"
                  className="transition-colors hover:text-brand-cyan"
                >
                  Changelog
                </a>
              </li>
              <li>
                <a
                  href={`${GITHUB_URL}/issues`}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="transition-colors hover:text-brand-cyan"
                >
                  Report an Issue
                </a>
              </li>
            </ul>
          </div>

          {/* Legal */}
          <div>
            <h4 className="mb-3 text-sm font-semibold text-text-primary">
              Legal
            </h4>
            <ul className="space-y-2 text-sm text-text-muted">
              <li>MIT License</li>
              <li>&copy; {new Date().getFullYear()} sqail</li>
            </ul>
          </div>
        </div>

        <div className="mt-8 border-t border-border pt-6 text-center text-xs text-text-dim">
          Built with Rust and egui
        </div>
      </div>
    </footer>
  );
}
