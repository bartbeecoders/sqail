import RELEASES from "../../releases.json";
import { VERSION } from "../lib/constants";

export default function Changelog() {
  return (
    <section id="changelog" className="py-24">
      <div className="mx-auto max-w-4xl px-6">
        <h2 className="mb-4 text-center text-3xl font-bold text-text-primary sm:text-4xl">
          Changelog
        </h2>
        <p className="mb-12 text-center text-text-muted">
          What&apos;s new in each release.
        </p>

        <div className="space-y-10">
          {RELEASES.map((release) => (
            <div
              key={release.version}
              className="rounded-xl border border-border bg-bg-section p-6 sm:p-8"
            >
              <div className="mb-4 flex items-center gap-3">
                <h3 className="text-xl font-bold text-text-primary">
                  v{release.version}
                </h3>
                {release.version === VERSION && (
                  <span className="rounded-full bg-brand-cyan/15 px-2.5 py-0.5 text-xs font-semibold text-brand-cyan">
                    latest
                  </span>
                )}
              </div>

              <div className="space-y-5">
                {release.sections.map((section) => (
                  <div key={section.title}>
                    <h4 className="mb-2 text-sm font-semibold text-brand-yellow">
                      {section.title}
                    </h4>
                    <ul className="space-y-1.5 pl-4">
                      {section.items.map((item, i) => (
                        <li
                          key={i}
                          className="relative text-sm leading-relaxed text-text-muted before:absolute before:-left-3 before:top-2.5 before:h-1 before:w-1 before:rounded-full before:bg-text-dim"
                        >
                          {item}
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
