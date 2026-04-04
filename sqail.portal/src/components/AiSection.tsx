import { Bot, Sparkles, FileText, Wand2 } from "lucide-react";
import { AI_PROVIDERS } from "../lib/constants";

const AI_FEATURES = [
  {
    icon: <Wand2 size={20} />,
    title: "Natural Language to SQL",
    description: "Describe what you need in plain English and get production-ready SQL.",
  },
  {
    icon: <Sparkles size={20} />,
    title: "Query Explanation",
    description: "Paste a complex query and get a clear, line-by-line breakdown.",
  },
  {
    icon: <Bot size={20} />,
    title: "Query Optimization",
    description: "Get suggestions to make your queries faster and more efficient.",
  },
  {
    icon: <FileText size={20} />,
    title: "Documentation Generation",
    description: "Auto-generate metadata and documentation for your entire schema.",
  },
];

export default function AiSection() {
  return (
    <section id="ai" className="bg-bg-section py-24">
      <div className="mx-auto max-w-6xl px-6">
        <div className="grid gap-16 lg:grid-cols-2">
          {/* Left — text */}
          <div>
            <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
              AI-powered{" "}
              <span className="text-brand-yellow">SQL intelligence</span>
            </h2>
            <p className="mb-8 text-text-muted">
              SQaiL integrates with your favorite AI provider to supercharge your
              database workflow. Write queries faster, understand complex SQL
              instantly, and document your schema automatically.
            </p>

            <div className="space-y-5">
              {AI_FEATURES.map((f) => (
                <div key={f.title} className="flex gap-4">
                  <div className="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-brand-yellow/10 text-brand-yellow">
                    {f.icon}
                  </div>
                  <div>
                    <h3 className="mb-1 font-semibold text-text-primary">
                      {f.title}
                    </h3>
                    <p className="text-sm text-text-muted">{f.description}</p>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* Right — providers + image */}
          <div className="flex flex-col justify-center">
            <div className="rounded-2xl border border-border bg-bg-primary p-8">
              <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-text-dim">
                Works with your AI provider
              </h3>
              <p className="mb-6 text-sm text-text-muted">
                Bring your own API key. SQaiL supports 7 providers out of the box,
                plus any OpenAI-compatible endpoint.
              </p>
              <div className="flex flex-wrap gap-2">
                {AI_PROVIDERS.map((provider) => (
                  <span
                    key={provider}
                    className="rounded-full border border-border bg-bg-section px-3 py-1.5 text-xs font-medium text-text-primary transition-colors hover:border-brand-yellow/40"
                  >
                    {provider}
                  </span>
                ))}
              </div>
            </div>

            {/* Visual accent */}
            <div className="mt-6 rounded-2xl border border-border bg-gradient-to-br from-brand-yellow/5 to-brand-cyan/5 p-8 text-center">
              <div className="mb-3 text-4xl font-bold text-brand-yellow">7+</div>
              <p className="text-sm text-text-muted">
                AI providers supported — from cloud APIs to local models
              </p>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
