import { useState } from "react";

interface Shot {
  src: string;
  alt: string;
  label: string;
}

const SHOTS: Shot[] = [
  {
    src: "/screenshots/editor.png",
    alt: "sqail SQL editor with query results",
    label: "Editor",
  },
  {
    src: "/screenshots/connections.png",
    alt: "sqail connection manager with multiple databases",
    label: "Connections",
  },
  {
    src: "/screenshots/ai.png",
    alt: "sqail AI sidebar generating SQL from natural language",
    label: "AI",
  },
  {
    src: "/screenshots/split.png",
    alt: "sqail split editor with two query panes",
    label: "Split Editor",
  },
  {
    src: "/screenshots/light.png",
    alt: "sqail in light theme",
    label: "Light Theme",
  },
];

export default function Screenshots() {
  const [active, setActive] = useState(0);

  return (
    <section id="screenshots" className="py-24">
      <div className="mx-auto max-w-6xl px-6">
        <div className="mb-12 text-center">
          <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
            See it in action
          </h2>
          <p className="mx-auto max-w-2xl text-text-muted">
            Dark and light themes, split editor, multi-database connections, and
            AI — all in under 20 MB.
          </p>
        </div>

        {/* Main image */}
        <div className="mb-6 overflow-hidden rounded-2xl border border-border bg-bg-section shadow-2xl shadow-brand-cyan/5">
          <img
            src={SHOTS[active].src}
            alt={SHOTS[active].alt}
            className="h-auto w-full"
          />
        </div>

        {/* Thumbnail strip */}
        <div className="flex justify-center gap-3">
          {SHOTS.map((shot, idx) => (
            <button
              key={shot.label}
              onClick={() => setActive(idx)}
              className={`rounded-lg border px-4 py-2 text-sm font-medium transition-colors ${
                idx === active
                  ? "border-brand-cyan bg-brand-cyan/10 text-brand-cyan"
                  : "border-border bg-bg-section text-text-muted hover:border-text-dim hover:text-text-primary"
              }`}
            >
              {shot.label}
            </button>
          ))}
        </div>
      </div>
    </section>
  );
}
