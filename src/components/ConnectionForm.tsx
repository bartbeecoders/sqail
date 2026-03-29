import { useState } from "react";
import { X, Loader2, CheckCircle2, XCircle } from "lucide-react";
import { cn } from "../lib/utils";
import { useConnectionStore } from "../stores/connectionStore";
import {
  type ConnectionConfig,
  type Driver,
  defaultConnection,
  defaultPort,
  DRIVER_LABELS,
} from "../types/connection";

interface ConnectionFormProps {
  initial?: ConnectionConfig;
  onClose: () => void;
}

type TestStatus = "idle" | "testing" | "success" | "error";

export default function ConnectionForm({ initial, onClose }: ConnectionFormProps) {
  const [form, setForm] = useState<ConnectionConfig>(initial ?? defaultConnection());
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testMessage, setTestMessage] = useState("");
  const [saving, setSaving] = useState(false);

  const { createConnection, updateConnection, testConnection } = useConnectionStore();
  const isEdit = !!initial;

  const set = <K extends keyof ConnectionConfig>(key: K, value: ConnectionConfig[K]) =>
    setForm((prev) => ({ ...prev, [key]: value }));

  const handleDriverChange = (driver: Driver) => {
    setForm((prev) => ({
      ...prev,
      driver,
      port: defaultPort(driver),
    }));
  };

  const handleTest = async () => {
    setTestStatus("testing");
    setTestMessage("");
    try {
      const msg = await testConnection(form);
      setTestStatus("success");
      setTestMessage(msg);
    } catch (e) {
      setTestStatus("error");
      setTestMessage(String(e));
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      if (isEdit) {
        await updateConnection(form);
      } else {
        await createConnection(form);
      }
      onClose();
    } catch (e) {
      setTestStatus("error");
      setTestMessage(String(e));
    } finally {
      setSaving(false);
    }
  };

  const isNetwork = form.driver !== "sqlite";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-full max-w-md rounded-lg border border-border bg-background p-5 shadow-xl">
        {/* Header */}
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-sm font-semibold">
            {isEdit ? "Edit Connection" : "New Connection"}
          </h2>
          <button
            onClick={onClose}
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={16} />
          </button>
        </div>

        <div className="space-y-3">
          {/* Name */}
          <Field label="Name">
            <input
              value={form.name}
              onChange={(e) => set("name", e.target.value)}
              placeholder="My Database"
              className="input"
            />
          </Field>

          {/* Driver */}
          <Field label="Driver">
            <div className="flex gap-1">
              {(Object.keys(DRIVER_LABELS) as Driver[]).map((d) => (
                <button
                  key={d}
                  onClick={() => handleDriverChange(d)}
                  className={cn(
                    "flex-1 rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                    form.driver === d
                      ? "bg-primary text-primary-foreground"
                      : "bg-muted text-muted-foreground hover:bg-accent",
                  )}
                >
                  {DRIVER_LABELS[d]}
                </button>
              ))}
            </div>
          </Field>

          {/* Network fields */}
          {isNetwork && (
            <>
              <div className="flex gap-2">
                <Field label="Host" className="flex-1">
                  <input
                    value={form.host}
                    onChange={(e) => set("host", e.target.value)}
                    placeholder="localhost"
                    className="input"
                  />
                </Field>
                <Field label="Port" className="w-24">
                  <input
                    type="number"
                    value={form.port || ""}
                    onChange={(e) => set("port", Number(e.target.value))}
                    className="input"
                  />
                </Field>
              </div>
              <Field label="Database">
                <input
                  value={form.database}
                  onChange={(e) => set("database", e.target.value)}
                  placeholder="mydb"
                  className="input"
                />
              </Field>
              <div className="flex gap-2">
                <Field label="User" className="flex-1">
                  <input
                    value={form.user}
                    onChange={(e) => set("user", e.target.value)}
                    placeholder="postgres"
                    className="input"
                  />
                </Field>
                <Field label="Password" className="flex-1">
                  <input
                    type="password"
                    value={form.password}
                    onChange={(e) => set("password", e.target.value)}
                    className="input"
                  />
                </Field>
              </div>
            </>
          )}

          {/* SQLite file path */}
          {!isNetwork && (
            <Field label="Database File">
              <input
                value={form.filePath}
                onChange={(e) => set("filePath", e.target.value)}
                placeholder="/path/to/database.db"
                className="input"
              />
            </Field>
          )}

          {/* Color */}
          <Field label="Color (optional)">
            <input
              type="color"
              value={form.color || "#6366f1"}
              onChange={(e) => set("color", e.target.value)}
              className="h-8 w-12 cursor-pointer rounded border border-border bg-transparent"
            />
          </Field>

          {/* Test result */}
          {testStatus !== "idle" && (
            <div
              className={cn(
                "flex items-start gap-2 rounded-md p-2 text-xs",
                testStatus === "testing" && "bg-muted text-muted-foreground",
                testStatus === "success" && "bg-success/10 text-success",
                testStatus === "error" && "bg-destructive/10 text-destructive",
              )}
            >
              {testStatus === "testing" && <Loader2 size={14} className="mt-0.5 animate-spin" />}
              {testStatus === "success" && <CheckCircle2 size={14} className="mt-0.5" />}
              {testStatus === "error" && <XCircle size={14} className="mt-0.5 shrink-0" />}
              <span className="break-all">{testStatus === "testing" ? "Testing..." : testMessage}</span>
            </div>
          )}

          {/* Actions */}
          <div className="flex justify-end gap-2 pt-2">
            <button onClick={handleTest} disabled={testStatus === "testing"} className="btn-secondary">
              Test Connection
            </button>
            <button onClick={onClose} className="btn-secondary">
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={saving || !form.name}
              className="btn-primary"
            >
              {saving ? "Saving..." : isEdit ? "Update" : "Create"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Field({
  label,
  children,
  className,
}: {
  label: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <label className={cn("block", className)}>
      <span className="mb-1 block text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}
