import { useState } from "react";
import { Menu, X } from "lucide-react";
import { NAV_ITEMS, GITHUB_URL } from "../lib/constants";

export default function Navbar() {
  const [mobileOpen, setMobileOpen] = useState(false);

  return (
    <nav className="fixed top-0 left-0 right-0 z-50 border-b border-border bg-bg-primary/80 backdrop-blur-lg">
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-3">
        {/* Logo */}
        <a href="#" className="flex items-center gap-2.5">
          <img src="/icon.png" alt="SQaiL" className="h-8 w-8" />
          <span className="text-lg font-bold text-text-primary">SQaiL</span>
        </a>

        {/* Desktop nav */}
        <div className="hidden items-center gap-8 md:flex">
          {NAV_ITEMS.map((item) => (
            <a
              key={item.href}
              href={item.href}
              className="text-sm text-text-muted transition-colors hover:text-brand-cyan"
            >
              {item.label}
            </a>
          ))}
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="text-sm text-text-muted transition-colors hover:text-brand-cyan"
          >
            GitHub
          </a>
          <a
            href="#download"
            className="rounded-lg bg-brand-cyan px-4 py-2 text-sm font-semibold text-bg-primary transition-colors hover:bg-brand-cyan/85"
          >
            Download
          </a>
        </div>

        {/* Mobile toggle */}
        <button
          onClick={() => setMobileOpen(!mobileOpen)}
          className="text-text-muted md:hidden"
        >
          {mobileOpen ? <X size={22} /> : <Menu size={22} />}
        </button>
      </div>

      {/* Mobile menu */}
      {mobileOpen && (
        <div className="border-t border-border bg-bg-primary px-6 pb-4 md:hidden">
          {NAV_ITEMS.map((item) => (
            <a
              key={item.href}
              href={item.href}
              onClick={() => setMobileOpen(false)}
              className="block py-2 text-sm text-text-muted transition-colors hover:text-brand-cyan"
            >
              {item.label}
            </a>
          ))}
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="block py-2 text-sm text-text-muted transition-colors hover:text-brand-cyan"
          >
            GitHub
          </a>
          <a
            href="#download"
            onClick={() => setMobileOpen(false)}
            className="mt-2 inline-block rounded-lg bg-brand-cyan px-4 py-2 text-sm font-semibold text-bg-primary"
          >
            Download
          </a>
        </div>
      )}
    </nav>
  );
}
