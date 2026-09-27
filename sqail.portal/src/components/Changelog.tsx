import type { ReactNode } from "react";
import CHANGELOG from "../../../CHANGELOG.md?raw";
import { CODEBERG_URL } from "../lib/constants";

interface Section {
  title: string;
  items: string[];
}

interface Entry {
  version: string;
  date: string;
  intro: string;
  sections: Section[];
}

/** Released versions from the repository's CHANGELOG.md ("Unreleased" is skipped). */
function parse(md: string): Entry[] {
  const entries: Entry[] = [];
  let entry: Entry | null = null;
  let section: Section | null = null;
  for (const line of md.split("\n")) {
    const h2 = line.match(/^## (\S+)(?: — (.*))?$/);
    if (h2) {
      entry = /^\d/.test(h2[1]) ? { version: h2[1], date: h2[2] ?? "", intro: "", sections: [] } : null;
      if (entry) entries.push(entry);
      section = null;
    } else if (!entry) {
      continue;
    } else if (line.startsWith("### ")) {
      section = { title: line.slice(4).trim(), items: [] };
      entry.sections.push(section);
    } else if (line.startsWith("- ") && section) {
      section.items.push(line.slice(2).trim());
    } else if (/^\s+\S/.test(line) && section?.items.length) {
      section.items[section.items.length - 1] += " " + line.trim();
    } else if (line.trim() && !section) {
      entry.intro += (entry.intro ? " " : "") + line.trim();
    }
  }
  return entries;
}

/** `code`, **bold** and [links](url); relative links point into the repository. */
function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /`([^`]+)`|\*\*([^*]+)\*\*|\[([^\]]+)\]\(([^)]+)\)/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    out.push(text.slice(last, m.index));
    const key = m.index;
    if (m[1] !== undefined) {
      out.push(
        <code key={key} className="rounded bg-bg-card px-1 py-0.5 text-xs text-text-primary">
          {m[1]}
        </code>,
      );
    } else if (m[2] !== undefined) {
      out.push(
        <strong key={key} className="text-text-primary">
          {m[2]}
        </strong>,
      );
    } else {
      const href = /^https?:/.test(m[4]) ? m[4] : `${CODEBERG_URL}/src/branch/main/${m[4]}`;
      out.push(
        <a key={key} href={href} className="text-brand-cyan hover:underline">
          {m[3]}
        </a>,
      );
    }
    last = m.index + m[0].length;
  }
  out.push(text.slice(last));
  return out;
}

const RELEASES = parse(CHANGELOG);

export default function Changelog() {
  return (
    <section id="changelog" className="py-24">
      <div className="mx-auto max-w-4xl px-6">
        <h2 className="mb-4 text-center text-3xl font-bold text-text-primary sm:text-4xl">Changelog</h2>
        <p className="mb-12 text-center text-text-muted">What&apos;s new in each release.</p>

        <div className="space-y-10">
          {RELEASES.map((release, idx) => (
            <div key={release.version} className="rounded-xl border border-border bg-bg-section p-6 sm:p-8">
              <div className="mb-4 flex flex-wrap items-center gap-3">
                <h3 className="text-xl font-bold text-text-primary">v{release.version}</h3>
                {idx === 0 && (
                  <span className="rounded-full bg-brand-cyan/15 px-2.5 py-0.5 text-xs font-semibold text-brand-cyan">
                    latest
                  </span>
                )}
                {release.date && <span className="text-sm text-text-dim">{release.date}</span>}
              </div>
              {release.intro && <p className="mb-5 text-sm text-text-muted">{inline(release.intro)}</p>}

              <div className="space-y-5">
                {release.sections.map((section) => (
                  <div key={section.title}>
                    <h4 className="mb-2 text-sm font-semibold text-brand-yellow">{section.title}</h4>
                    <ul className="space-y-1.5 pl-4">
                      {section.items.map((item, i) => (
                        <li
                          key={i}
                          className="relative text-sm leading-relaxed text-text-muted before:absolute before:top-2.5 before:-left-3 before:h-1 before:w-1 before:rounded-full before:bg-text-dim"
                        >
                          {inline(item)}
                        </li>
                      ))}
                    </ul>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
