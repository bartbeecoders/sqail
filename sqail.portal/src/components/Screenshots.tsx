import { useState } from "react";

interface Shot {
  src: string;
  alt: string;
  label: string;
}

const SHOTS: Shot[] = [
  { src: "/screenshots/results.png", alt: "sqail editor with a query and its result grid", label: "Editor" },
  { src: "/screenshots/million-rows.png", alt: "a million PostgreSQL rows streamed into the grid", label: "Big results" },
  { src: "/screenshots/completion.png", alt: "schema-aware completion while typing", label: "Completion" },
  { src: "/screenshots/editing.png", alt: "editing rows of a table in the result grid", label: "Edit data" },
  { src: "/screenshots/plan.png", alt: "the estimated query plan as a tree", label: "Plans" },
  { src: "/screenshots/connection.png", alt: "the connection form", label: "Connections" },
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
            Streaming results, completion that knows your schema, in-place
            editing and query plans, in a download of under 20 MB.
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
        <div className="flex flex-wrap justify-center gap-3">
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
