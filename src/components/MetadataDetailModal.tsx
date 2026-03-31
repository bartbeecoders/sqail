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

const TABS = ["Overview", "Columns", "Usage & Relations"] as const;
type Tab = (typeof TABS)[number];

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
  const [activeTab, setActiveTab] = useState<Tab>("Overview");

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
      <div className="flex h-[80vh] max-h-[720px] w-full max-w-3xl flex-col rounded-lg border border-border bg-background shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border px-6 py-4">
          <div className="flex items-center gap-3">
            <h2 className="text-base font-semibold">
              {entry.schemaName}.{entry.objectName}
            </h2>
            <span
              className={cn(
                "rounded-md px-2 py-0.5 text-[11px] font-medium",
                typeBadgeColor[entry.objectType] ?? "bg-muted text-muted-foreground",
              )}
            >
              {entry.objectType}
            </span>
          </div>
          <button
            onClick={onClose}
            className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={16} />
          </button>
        </div>

        {/* Tabs */}
        <div className="flex gap-0 border-b border-border px-6">
          {TABS.map((tab) => (
            <button
              key={tab}
              onClick={() => setActiveTab(tab)}
              className={cn(
                "relative px-4 py-2.5 text-xs font-medium transition-colors",
                activeTab === tab
                  ? "text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {tab}
              {tab === "Columns" && meta.columns.length > 0 && (
                <span className="ml-1.5 text-[10px] opacity-50">{meta.columns.length}</span>
              )}
              {activeTab === tab && (
                <span className="absolute bottom-0 left-0 right-0 h-0.5 bg-primary" />
              )}
            </button>
          ))}
        </div>

        {/* Tab content */}
        <div className="flex-1 overflow-y-auto px-6 py-5">
          {activeTab === "Overview" && (
            <div className="space-y-5">
              <Section label="Description">
                <textarea
                  value={meta.description}
                  onChange={(e) => update({ description: e.target.value })}
                  rows={5}
                  className="input w-full resize-vertical text-sm leading-relaxed"
                  placeholder="Describe what this object is used for..."
                />
              </Section>

              <div className="rounded-md border border-border bg-muted/30 px-4 py-3">
                <div className="grid grid-cols-2 gap-x-6 gap-y-2 text-xs">
                  <div>
                    <span className="text-muted-foreground">Generated</span>
                    <p className="font-medium">{new Date(entry.generatedAt).toLocaleString()}</p>
                  </div>
                  <div>
                    <span className="text-muted-foreground">Last updated</span>
                    <p className="font-medium">
                      {entry.updatedAt !== entry.generatedAt
                        ? new Date(entry.updatedAt).toLocaleString()
                        : "—"}
                    </p>
                  </div>
                </div>
              </div>
            </div>
          )}

          {activeTab === "Columns" && (
            <div className="space-y-1">
              {meta.columns.length === 0 ? (
                <p className="py-8 text-center text-sm text-muted-foreground">
                  No column metadata available.
                </p>
              ) : (
                <>
                  {/* Column header */}
                  <div className="flex items-center gap-3 px-1 pb-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                    <span className="w-40 shrink-0">Column</span>
                    <span className="flex-1">Description</span>
                  </div>
                  {meta.columns.map((col, i) => (
                    <div
                      key={col.name}
                      className={cn(
                        "flex items-start gap-3 rounded-md px-1 py-2",
                        i % 2 === 0 && "bg-muted/30",
                      )}
                    >
                      <span className="mt-1 w-40 shrink-0 truncate font-mono text-xs font-medium">
                        {col.name}
                      </span>
                      <textarea
                        value={col.description}
                        onChange={(e) => updateColumn(i, e.target.value)}
                        rows={1}
                        className="input flex-1 resize-none text-xs"
                        placeholder="Describe this column..."
                      />
                    </div>
                  ))}
                </>
              )}
            </div>
          )}

          {activeTab === "Usage & Relations" && (
            <div className="space-y-6">
              <Section label="Example Usage">
                <textarea
                  value={meta.exampleUsage}
                  onChange={(e) => update({ exampleUsage: e.target.value })}
                  rows={5}
                  className="input w-full resize-vertical font-mono text-xs leading-relaxed"
                  placeholder="SELECT * FROM ..."
                />
              </Section>

              <Section label="Related Objects">
                <EditableList
                  items={meta.relatedObjects}
                  onChange={(i, v) => updateListItem("relatedObjects", i, v)}
                  onAdd={() => addListItem("relatedObjects")}
                  onRemove={(i) => removeListItem("relatedObjects", i)}
                  placeholder="schema.table_name"
                />
              </Section>

              <Section label="Dependencies">
                <EditableList
                  items={meta.dependencies}
                  onChange={(i, v) => updateListItem("dependencies", i, v)}
                  onAdd={() => addListItem("dependencies")}
                  onRemove={(i) => removeListItem("dependencies", i)}
                  placeholder="schema.object_name"
                />
              </Section>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 border-t border-border px-6 py-4">
          <button
            onClick={handleRegenerate}
            disabled={regenerating}
            className="flex items-center gap-1.5 rounded-md border border-border px-4 py-2 text-xs font-medium text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            {regenerating ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <RefreshCw size={14} />
            )}
            Regenerate
          </button>
          <button
            onClick={handleSave}
            disabled={!dirty || saving}
            className="flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {saving ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Save size={14} />
            )}
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="mb-2 block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
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
  placeholder,
}: {
  items: string[];
  onChange: (index: number, value: string) => void;
  onAdd: () => void;
  onRemove: (index: number) => void;
  placeholder?: string;
}) {
  return (
    <div className="space-y-2">
      {items.map((item, i) => (
        <div key={i} className="flex items-center gap-2">
          <input
            type="text"
            value={item}
            onChange={(e) => onChange(i, e.target.value)}
            className="input flex-1 text-xs"
            placeholder={placeholder}
          />
          <button
            onClick={() => onRemove(i)}
            className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-destructive"
          >
            <Trash2 size={12} />
          </button>
        </div>
      ))}
      <button
        onClick={onAdd}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      >
        <Plus size={12} />
        Add
      </button>
    </div>
  );
}
