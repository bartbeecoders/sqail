import { DATABASES } from "../lib/constants";

export default function DatabaseSupport() {
  return (
    <section id="databases" className="py-24">
      <div className="mx-auto max-w-6xl px-6">
        <div className="mb-16 text-center">
          <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
            One app, four databases
          </h2>
          <p className="mx-auto max-w-2xl text-text-muted">
            SQaiL speaks your database's dialect — with syntax highlighting,
            autocomplete, and validation tuned to each engine.
          </p>
        </div>

        <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-4">
          {DATABASES.map((db) => (
            <div
              key={db.name}
              className="group rounded-xl border border-border bg-bg-section p-6 text-center transition-colors hover:border-brand-cyan/40"
            >
              <div
                className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-full text-2xl font-bold text-white"
                style={{ backgroundColor: db.color }}
              >
                {db.name[0]}
              </div>
              <h3 className="mb-2 font-semibold text-text-primary">
                {db.name}
              </h3>
              <p className="text-sm leading-relaxed text-text-muted">
                {db.description}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
