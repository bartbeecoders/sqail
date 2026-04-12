import {
  Zap,
  Sparkles,
  GitBranch,
  ShieldCheck,
  Database,
} from "lucide-react";
import { FEATURES } from "../lib/constants";

const ICON_MAP: Record<string, React.ReactNode> = {
  Zap: <Zap size={24} />,
  Sparkles: <Sparkles size={24} />,
  GitBranch: <GitBranch size={24} />,
  ShieldCheck: <ShieldCheck size={24} />,
  Database: <Database size={24} />,
};

// Smart and Private use yellow per brand-guide §3 ("yellow owns intelligence / AI").
// Fast, Free, Universal use cyan.
const ACCENT_MAP: Record<string, "cyan" | "yellow"> = {
  Fast: "cyan",
  Smart: "yellow",
  Free: "cyan",
  Private: "yellow",
  Universal: "cyan",
};

export default function Features() {
  return (
    <section id="features" className="py-24">
      <div className="mx-auto max-w-6xl px-6">
        <div className="mb-16 text-center">
          <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
            Five things sqail does well
          </h2>
          <p className="mx-auto max-w-2xl text-text-muted">
            Everything else is intentionally out of scope. We'd rather ship one
            small, fast tool than a bloated Swiss Army knife.
          </p>
        </div>

        <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
          {FEATURES.map((feature, idx) => {
            const accent = ACCENT_MAP[feature.title] ?? "cyan";
            const isCyan = accent === "cyan";
            const isLast = idx === FEATURES.length - 1;
            return (
              <div
                key={feature.title}
                className={`group rounded-xl border border-border bg-bg-section p-6 transition-colors ${
                  isCyan
                    ? "hover:border-brand-cyan/40"
                    : "hover:border-brand-yellow/40"
                } hover:bg-bg-card ${
                  // Make the last card (Universal) span two columns on lg to fill the row
                  isLast ? "lg:col-span-1" : ""
                }`}
              >
                <div
                  className={`mb-4 inline-flex rounded-lg p-2.5 transition-colors ${
                    isCyan
                      ? "bg-brand-cyan/10 text-brand-cyan group-hover:bg-brand-cyan/20"
                      : "bg-brand-yellow/10 text-brand-yellow group-hover:bg-brand-yellow/20"
                  }`}
                >
                  {ICON_MAP[feature.icon]}
                </div>
                <h3 className="mb-1 text-lg font-semibold text-text-primary">
                  {feature.title}
                </h3>
                <p
                  className={`mb-3 text-sm font-medium ${
                    isCyan ? "text-brand-cyan" : "text-brand-yellow"
                  }`}
                >
                  {feature.headline}
                </p>
                <p className="text-sm leading-relaxed text-text-muted">
                  {feature.description}
                </p>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
