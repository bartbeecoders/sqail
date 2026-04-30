import { useMemo, useState } from "react";
import { Copy, Check } from "lucide-react";
import { formatResultAsText } from "../lib/formatTextResults";
import type { QueryColumn } from "../types/query";

type CellValue = string | number | boolean | null;

interface Props {
  columns: QueryColumn[];
  rows: CellValue[][];
}

export default function TextResultsView({ columns, rows }: Props) {
  const text = useMemo(() => formatResultAsText(columns, rows), [columns, rows]);
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <div className="relative flex flex-1 flex-col overflow-hidden">
      <button
        onClick={handleCopy}
        className="absolute right-3 top-2 z-10 flex items-center gap-1 rounded-md border border-border bg-background/90 px-2 py-1 text-[10px] text-muted-foreground shadow-sm hover:bg-accent hover:text-accent-foreground"
        title="Copy text"
      >
        {copied ? <Check size={11} /> : <Copy size={11} />}
        {copied ? "Copied" : "Copy"}
      </button>
      <div className="flex-1 overflow-auto bg-background">
        <pre className="m-0 whitespace-pre p-3 font-mono text-xs leading-snug">
          {text}
        </pre>
      </div>
    </div>
  );
}
