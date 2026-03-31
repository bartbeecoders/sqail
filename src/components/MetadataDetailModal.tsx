import { useState } from "react";
import { X, Save, RefreshCw, Loader2, Plus, Trash2 } from "lucide-react";
import { cn } from "../lib/utils";
import { useMetadataStore } from "../stores/metadataStore";
import type { ObjectMetadata, GeneratedMetadata } from "../types/metadata";

interface MetadataDetailModalProps {
  entry: ObjectMetadata;
  connectionId: string;
  onClose: () => void;
}

export default function MetadataDetailModal({
  entry,
  connectionId,
  onClose,
}: MetadataDetailModalProps) {
  const { updateEntry, generateSingle } = useMetadataStore();
  const [meta, setMeta] = useState<GeneratedMetadata>({ ...entry.metadata });
  const [saving, setSaving] = useState(false);
  const [regenerating, setRegenerating] = useState(false);
  const [dirty, setDirty] = useState(false);

  const update = (patch: Partial<GeneratedMetadata>) => {
    setMeta((prev) => ({ ...prev, ...patch }));
    setDirty(true);
  };

  const updateColumn = (index: number, description: string) => {
    const columns = [...meta.columns];
    columns[index] = { ...columns[index], description };
    update({ columns });
  };

  const updateListItem = (
    field: "relatedObjects" | "dependencies",
    index: number,
    value: string,
  ) => {
    const list = [...meta[field]];
    list[index] = value;
    update({ [field]: list });
  };

  const addListItem = (field: "relatedObjects" | "dependencies") => {
    update({ [field]: [...meta[field], ""] });
  };

  const removeListItem = (field: "relatedObjects" | "dependencies", index: number) => {
    update({ [field]: meta[field].filter((_, i) => i !== index) });
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await updateEntry({
        ...entry,
        metadata: meta,
      });
      setDirty(false);
    } catch (e) {
      console.error("Failed to save:", e);
    } finally {
      setSaving(false);
    }
  };

  const handleRegenerate = async () => {
    setRegenerating(true);
    try {
      await generateSingle(
        connectionId,
        entry.schemaName,
        entry.objectName,
        entry.objectType,
      );
      // Reload the entry from store
      const updated = useMetadataStore
        .getState()
        .entries.find((e) => e.objectName === entry.objectName && e.schemaName === entry.schemaName);
      if (updated) {
        setMeta({ ...updated.metadata });
        setDirty(false);
      }
    } catch (e) {
      console.error("Failed to regenerate:", e);
    } finally {
      setRegenerating(false);
    }
  };

  const typeBadgeColor: Record<string, string> = {
    table: "bg-blue-500/15 text-blue-600 dark:text-blue-400",
    view: "bg-purple-500/15 text-purple-600 dark:text-purple-400",
    function: "bg-green-500/15 text-green-600 dark:text-green-400",
    procedure: "bg-orange-500/15 text-orange-600 dark:text-orange-400",
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="flex h-[600px] w-full max-w-2xl flex-col rounded-lg border border-border bg-background shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border px-5 py-3">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold">
              {entry.schemaName}.{entry.objectName}
            </h2>
            <span
              className={cn(
                "rounded px-1.5 py-0.5 text-[10px] font-medium",
                typeBadgeColor[entry.objectType] ?? "bg-muted text-muted-foreground",
              )}
            >
              {entry.objectType}
            </span>
          </div>
          <button
            onClick={onClose}
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={16} />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-5 py-4 space-y-4">
          {/* Description */}
          <Section label="Description">
            <textarea
              value={meta.description}
              onChange={(e) => update({ description: e.target.value })}
              rows={3}
              className="input w-full resize-none text-xs"
            />
          </Section>

          {/* Column descriptions */}
          {meta.columns.length > 0 && (
            <Section label="Columns">
              <div className="space-y-2">
                {meta.columns.map((col, i) => (
                  <div key={col.name} className="flex items-start gap-2">
                    <span className="mt-1.5 w-28 shrink-0 truncate text-[11px] font-mono text-muted-foreground">
                      {col.name}
                    </span>
                    <textarea
                      value={col.description}
                      onChange={(e) => updateColumn(i, e.target.value)}
                      rows={1}
                      className="input flex-1 resize-none text-[11px]"
                    />
                  </div>
                ))}
              </div>
            </Section>
          )}

          {/* Example usage */}
          <Section label="Example Usage">
            <textarea
              value={meta.exampleUsage}
              onChange={(e) => update({ exampleUsage: e.target.value })}
              rows={3}
              className="input w-full resize-none font-mono text-[11px]"
            />
          </Section>

          {/* Related objects */}
          <Section label="Related Objects">
            <EditableList
              items={meta.relatedObjects}
              onChange={(i, v) => updateListItem("relatedObjects", i, v)}
              onAdd={() => addListItem("relatedObjects")}
              onRemove={(i) => removeListItem("relatedObjects", i)}
            />
          </Section>

          {/* Dependencies */}
          <Section label="Dependencies">
            <EditableList
              items={meta.dependencies}
              onChange={(i, v) => updateListItem("dependencies", i, v)}
              onAdd={() => addListItem("dependencies")}
              onRemove={(i) => removeListItem("dependencies", i)}
            />
          </Section>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-border px-5 py-3">
          <div className="text-[10px] text-muted-foreground">
            Generated: {new Date(entry.generatedAt).toLocaleString()}
            {entry.updatedAt !== entry.generatedAt && (
              <> | Updated: {new Date(entry.updatedAt).toLocaleString()}</>
            )}
          </div>
          <div className="flex gap-2">
            <button
              onClick={handleRegenerate}
              disabled={regenerating}
              className="flex items-center gap-1 rounded-md border border-border px-3 py-1.5 text-[11px] font-medium text-muted-foreground hover:bg-accent disabled:opacity-50"
            >
              {regenerating ? (
                <Loader2 size={12} className="animate-spin" />
              ) : (
                <RefreshCw size={12} />
              )}
              Regenerate
            </button>
            <button
              onClick={handleSave}
              disabled={!dirty || saving}
              className="flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-[11px] font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              {saving ? (
                <Loader2 size={12} className="animate-spin" />
              ) : (
                <Save size={12} />
              )}
              Save
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  );
}

function EditableList({
  items,
  onChange,
  onAdd,
  onRemove,
}: {
  items: string[];
  onChange: (index: number, value: string) => void;
  onAdd: () => void;
  onRemove: (index: number) => void;
}) {
  return (
    <div className="space-y-1">
      {items.map((item, i) => (
        <div key={i} className="flex items-center gap-1">
          <input
            type="text"
            value={item}
            onChange={(e) => onChange(i, e.target.value)}
            className="input flex-1 text-[11px]"
          />
          <button
            onClick={() => onRemove(i)}
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-destructive"
          >
            <Trash2 size={11} />
          </button>
        </div>
      ))}
      <button
        onClick={onAdd}
        className="flex items-center gap-1 text-[10px] text-muted-foreground hover:text-foreground"
      >
        <Plus size={10} />
        Add
      </button>
    </div>
  );
}
