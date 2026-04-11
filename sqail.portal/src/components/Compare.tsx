import type { ReactNode } from "react";
import { Check, X, Minus } from "lucide-react";

type Cell = "yes" | "no" | "partial";

interface Row {
  feature: string;
  sqail: Cell;
  dbeaver: Cell;
  datagrip: Cell;
  tableplus: Cell;
  beekeeper: Cell;
  note?: string;
}

const ROWS: Row[] = [
  {
    feature: "Free & open source",
    sqail: "yes",
    dbeaver: "partial",
    datagrip: "no",
    tableplus: "no",
    beekeeper: "partial",
    note: "DBeaver & Beekeeper have paid tiers; DataGrip & TablePlus are paid.",
  },
  {
    feature: "Native (non-Electron) binary",
    sqail: "yes",
    dbeaver: "partial",
    datagrip: "no",
    tableplus: "yes",
    beekeeper: "no",
  },
  {
    feature: "Under 20 MB install",
    sqail: "yes",
    dbeaver: "no",
    datagrip: "no",
    tableplus: "partial",
    beekeeper: "no",
  },
  {
    feature: "Cross-platform (Linux / macOS / Windows)",
    sqail: "yes",
    dbeaver: "yes",
    datagrip: "yes",
    tableplus: "partial",
    beekeeper: "yes",
    note: "TablePlus is macOS-first with limited Linux support.",
  },
  {
    feature: "Built-in AI (NL→SQL, explain, optimize)",
    sqail: "yes",
    dbeaver: "no",
    datagrip: "partial",
    tableplus: "no",
    beekeeper: "no",
    note: "DataGrip offers AI as a paid JetBrains add-on.",
  },
  {
    feature: "Bring-your-own AI key (multi-provider)",
    sqail: "yes",
    dbeaver: "no",
    datagrip: "no",
    tableplus: "no",
    beekeeper: "no",
  },
  {
    feature: "No telemetry by default",
    sqail: "yes",
    dbeaver: "partial",
    datagrip: "no",
    tableplus: "partial",
    beekeeper: "partial",
  },
  {
    feature: "Monaco (VS Code) editor",
    sqail: "yes",
    dbeaver: "no",
    datagrip: "no",
    tableplus: "no",
    beekeeper: "yes",
  },
];

const CELL_STYLES: Record<Cell, { icon: ReactNode; label: string; className: string }> = {
  yes: {
    icon: <Check size={18} />,
    label: "Yes",
    className: "text-brand-cyan",
  },
  no: {
    icon: <X size={18} />,
    label: "No",
    className: "text-text-dim",
  },
  partial: {
    icon: <Minus size={18} />,
    label: "Partial",
    className: "text-brand-yellow",
  },
};

function CellMark({ value }: { value: Cell }) {
  const style = CELL_STYLES[value];
  return (
    <span
      className={`inline-flex items-center justify-center ${style.className}`}
      aria-label={style.label}
      title={style.label}
    >
      {style.icon}
    </span>
  );
}

export default function Compare() {
  return (
    <section id="compare" className="py-24">
      <div className="mx-auto max-w-6xl px-6">
        <div className="mb-12 text-center">
          <h2 className="mb-4 text-3xl font-bold text-text-primary sm:text-4xl">
            Why <span className="text-brand-cyan">sqail</span>?
          </h2>
          <p className="mx-auto max-w-2xl text-text-muted">
            The intersection of small, native, open source, and AI-first. No
            other SQL client hits all of these.
          </p>
        </div>

        <div className="overflow-x-auto rounded-2xl border border-border bg-bg-section">
          <table className="w-full min-w-[720px] text-left text-sm">
            <thead>
              <tr className="border-b border-border text-text-dim">
                <th className="px-5 py-4 font-medium">Feature</th>
                <th className="px-5 py-4 text-center font-semibold text-brand-cyan">
                  sqail
                </th>
                <th className="px-5 py-4 text-center font-medium">DBeaver</th>
                <th className="px-5 py-4 text-center font-medium">DataGrip</th>
                <th className="px-5 py-4 text-center font-medium">TablePlus</th>
                <th className="px-5 py-4 text-center font-medium">Beekeeper</th>
              </tr>
            </thead>
            <tbody>
              {ROWS.map((row) => (
                <tr
                  key={row.feature}
                  className="border-b border-border/60 last:border-b-0 hover:bg-bg-card/40"
                >
                  <td className="px-5 py-4 text-text-primary">
                    {row.feature}
                    {row.note && (
                      <div className="mt-1 text-xs text-text-dim">{row.note}</div>
                    )}
                  </td>
                  <td className="px-5 py-4 text-center">
                    <CellMark value={row.sqail} />
                  </td>
                  <td className="px-5 py-4 text-center">
                    <CellMark value={row.dbeaver} />
                  </td>
                  <td className="px-5 py-4 text-center">
                    <CellMark value={row.datagrip} />
                  </td>
                  <td className="px-5 py-4 text-center">
                    <CellMark value={row.tableplus} />
                  </td>
                  <td className="px-5 py-4 text-center">
                    <CellMark value={row.beekeeper} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <p className="mt-6 text-center text-xs text-text-dim">
          Comparison reflects publicly available information at the time of
          writing. Corrections welcome — file an issue on the repo.
        </p>
      </div>
    </section>
  );
}
