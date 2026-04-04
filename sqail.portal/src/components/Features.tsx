import {
  Monitor,
  Zap,
  Database,
  Code,
  History,
  Moon,
  Keyboard,
  GitBranch,
} from "lucide-react";
import { FEATURES } from "../lib/constants";

const ICON_MAP: Record<string, React.ReactNode> = {
  Monitor: <Monitor size={24} />,
  Zap: <Zap size={24} />,
  Database: <Database size={24} />,
  Code: <Code size={24} />,
  History: <History size={24} />,
  Moon: <Moon size={24} />,
  Keyboard: <Keyboard size={24} />,
  GitBranch: <GitBranch size={24} />,
};

export default function Features() {
  return (
    <section id="features" className="py-24">
      <div className="mx-auto max-w-6xl px-6">
        <div className="mb-16 text-center">
          <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
            Everything you need
          </h2>
          <p className="mx-auto max-w-2xl text-text-muted">
            A complete SQL workbench packed into a lightweight native app.
            No subscriptions, no cloud dependency, no compromises.
          </p>
        </div>

        <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-4">
          {FEATURES.map((feature) => (
            <div
              key={feature.title}
              className="group rounded-xl border border-border bg-bg-section p-6 transition-colors hover:border-brand-cyan/40 hover:bg-bg-card"
            >
              <div className="mb-4 inline-flex rounded-lg bg-brand-cyan/10 p-2.5 text-brand-cyan transition-colors group-hover:bg-brand-cyan/20">
                {ICON_MAP[feature.icon]}
              </div>
              <h3 className="mb-2 font-semibold text-text-primary">
                {feature.title}
              </h3>
              <p className="text-sm leading-relaxed text-text-muted">
                {feature.description}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
