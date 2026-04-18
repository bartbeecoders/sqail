import { Bot, Cpu, FileText, GraduationCap, HardDrive, Sparkles, Wand2 } from "lucide-react";
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

const LOCAL_AI_FEATURES = [
  {
    icon: <HardDrive size={20} />,
    title: "100% offline inline AI",
    description:
      "A bundled llama.cpp sidecar runs GGUF models locally for ghost-text and assistant flows. Auto-download on first enable — Vulkan on Windows/Linux, Metal on macOS.",
  },
  {
    icon: <Cpu size={20} />,
    title: "Curated model catalog",
    description:
      "Qwen2.5-Coder 0.5B / 7B / 14B, Qwen3.5 9B / 27B, Qwen3-Coder 30B-A3B MoE, and Qwen3.6 35B-A3B — VRAM-gated so you only see models your GPU can actually load.",
  },
  {
    icon: <GraduationCap size={20} />,
    title: "Database-tuned LoRA fine-tuning",
    description:
      "Pick a connection + a base model, and sqail builds a JSONL corpus from your schema, metadata, and sample rows, then runs a LoRA fine-tune on your local GPU. Activate the adapter with one click — no restart.",
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
              Cloud or local — sqail works with your favorite AI provider, or
              runs entirely on your machine. Write queries faster, understand
              complex SQL instantly, and document your schema automatically.
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

        {/* Local AI + training */}
        <div className="mt-20 border-t border-border pt-16">
          <div className="mb-12 max-w-3xl">
            <span className="mb-3 inline-flex items-center gap-2 rounded-full border border-brand-yellow/30 bg-brand-yellow/5 px-3 py-1 text-xs font-semibold text-brand-yellow">
              New in 0.6
            </span>
            <h3 className="mb-3 text-2xl font-bold text-text-primary sm:text-3xl">
              Local LLM &amp; on-device fine-tuning
            </h3>
            <p className="text-text-muted">
              No API key, no round-trip, no vendor lock-in. Run open-weight
              models on your own GPU — and teach one your specific schema with
              a LoRA fine-tune kicked off from the Settings UI.
            </p>
          </div>

          <div className="grid gap-6 md:grid-cols-3">
            {LOCAL_AI_FEATURES.map((f) => (
              <div
                key={f.title}
                className="rounded-xl border border-border bg-bg-primary p-6 transition-colors hover:border-brand-yellow/40"
              >
                <div className="mb-4 inline-flex rounded-lg bg-brand-yellow/10 p-2.5 text-brand-yellow">
                  {f.icon}
                </div>
                <h4 className="mb-2 text-base font-semibold text-text-primary">
                  {f.title}
                </h4>
                <p className="text-sm leading-relaxed text-text-muted">
                  {f.description}
                </p>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
