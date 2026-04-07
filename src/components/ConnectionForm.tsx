import { useState, useEffect, useRef, useCallback } from "react";
import { X, Loader2, CheckCircle2, XCircle, ExternalLink, ChevronDown, Link, FormInput } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../lib/utils";
import { useConnectionStore } from "../stores/connectionStore";
import {
  type ConnectionConfig,
  type Driver,
  type MssqlAuthMethod,
  defaultConnection,
  defaultPort,
  parseConnectionString,
  toConnectionString,
  DRIVER_LABELS,
  MSSQL_AUTH_LABELS,
} from "../types/connection";

interface ConnectionFormProps {
  initial?: ConnectionConfig;
  onClose: () => void;
}

type TestStatus = "idle" | "testing" | "success" | "error";

interface DeviceCodeInfo {
  userCode: string;
  deviceCode: string;
  verificationUri: string;
  expiresIn: number;
  interval: number;
}

export default function ConnectionForm({ initial, onClose }: ConnectionFormProps) {
  const [form, setForm] = useState<ConnectionConfig>(initial ?? defaultConnection());
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testMessage, setTestMessage] = useState("");
  const [saving, setSaving] = useState(false);
  const [entraSignedIn, setEntraSignedIn] = useState(false);
  const [entraLoading, setEntraLoading] = useState(false);
  const [deviceCode, setDeviceCode] = useState<DeviceCodeInfo | null>(null);
  const [connStringMode, setConnStringMode] = useState(false);
  const [connStringValue, setConnStringValue] = useState("");
  const [connStringError, setConnStringError] = useState("");

  const { createConnection, updateConnection, testConnection } = useConnectionStore();
  const isEdit = !!initial?.id;

  const set = <K extends keyof ConnectionConfig>(key: K, value: ConnectionConfig[K]) =>
    setForm((prev) => ({ ...prev, [key]: value }));

  const handleDriverChange = (driver: Driver) => {
    setForm((prev) => ({
      ...prev,
      driver,
      port: defaultPort(driver),
    }));
  };

  const handleEntraSignIn = async () => {
    setEntraLoading(true);
    setEntraSignedIn(false);
    setTestStatus("idle");

    // Ensure the form has an ID so the token can be stored against it
    let connId = form.id;
    if (!connId) {
      connId = crypto.randomUUID();
      setForm((prev) => ({ ...prev, id: connId }));
    }

    try {
      // Step 1: Start device code flow
      const dcInfo = await invoke<DeviceCodeInfo>("start_entra_login", {
        tenantId: form.tenantId,
        azureClientId: form.azureClientId,
      });
      setDeviceCode(dcInfo);

      // Step 2: Poll for token (blocks until user completes auth)
      await invoke<void>("poll_entra_token", {
        connectionId: connId,
        tenantId: form.tenantId,
        azureClientId: form.azureClientId,
        deviceCode: dcInfo.deviceCode,
      });

      setDeviceCode(null);
      setEntraSignedIn(true);
    } catch (e) {
      setDeviceCode(null);
      setTestStatus("error");
      setTestMessage(String(e));
    } finally {
      setEntraLoading(false);
    }
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

  const isDbService = form.driver === "dbservice";
  const isNetwork = form.driver !== "sqlite" && !isDbService;
  const supportsDbList = form.driver === "postgres" || form.driver === "mysql";

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

          {/* Mode toggle */}
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => {
                if (!connStringMode) {
                  // Switching to connection string mode — build from current form
                  setConnStringValue(toConnectionString(form));
                  setConnStringError("");
                }
                setConnStringMode(!connStringMode);
              }}
              className={cn(
                "flex items-center gap-1.5 rounded px-2 py-1 text-[11px] font-medium transition-colors",
                connStringMode
                  ? "bg-primary/10 text-primary"
                  : "text-muted-foreground hover:bg-accent hover:text-foreground",
              )}
            >
              {connStringMode ? <FormInput size={12} /> : <Link size={12} />}
              {connStringMode ? "Switch to form" : "Use connection string"}
            </button>
          </div>

          {connStringMode ? (
            <>
              <Field label="Connection String">
                <textarea
                  value={connStringValue}
                  onChange={(e) => {
                    setConnStringValue(e.target.value);
                    setConnStringError("");
                  }}
                  placeholder="postgresql://user:password@host:5432/database"
                  className="input font-mono text-[11px] resize-none"
                  rows={3}
                />
              </Field>
              {connStringError && (
                <div className="flex items-start gap-2 rounded-md p-2 text-xs bg-destructive/10 text-destructive">
                  <XCircle size={14} className="mt-0.5 shrink-0" />
                  <span>{connStringError}</span>
                </div>
              )}
              <button
                type="button"
                onClick={() => {
                  try {
                    const parsed = parseConnectionString(connStringValue);
                    setForm((prev) => ({
                      ...prev,
                      ...parsed,
                      name: prev.name || parsed.database || parsed.host || "",
                    }));
                    setConnStringError("");
                    setConnStringMode(false);
                  } catch (e) {
                    setConnStringError(e instanceof Error ? e.message : String(e));
                  }
                }}
                className="btn-secondary w-full text-xs"
              >
                Apply
              </button>
            </>
          ) : (
            <>
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

              {/* User/Password — render before Database so credentials are available for DB listing */}
              {form.driver !== "mssql" && (
                <div className="flex gap-2">
                  <Field label="User" className="flex-1">
                    <input
                      value={form.user}
                      onChange={(e) => set("user", e.target.value)}
                      placeholder={form.driver === "postgres" ? "postgres" : "root"}
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
              )}
              {form.driver === "mssql" && form.mssqlAuthMethod === "sql_server" && (
                <div className="flex gap-2">
                  <Field label="User" className="flex-1">
                    <input
                      value={form.user}
                      onChange={(e) => set("user", e.target.value)}
                      placeholder="sa"
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
              )}

              {/* Database — combobox for PG/MySQL, plain input for others */}
              <Field label="Database">
                {supportsDbList ? (
                  <DatabaseCombobox
                    value={form.database}
                    onChange={(v) => set("database", v)}
                    driver={form.driver}
                    host={form.host}
                    port={form.port}
                    user={form.user}
                    password={form.password}
                    sslMode={form.sslMode}
                    placeholder={form.driver === "postgres" ? "postgres" : "mydb"}
                  />
                ) : (
                  <input
                    value={form.database}
                    onChange={(e) => set("database", e.target.value)}
                    placeholder="mydb"
                    className="input"
                  />
                )}
              </Field>

              {form.driver === "mssql" && (
                <>
                  <Field label="Authentication">
                    <div className="flex gap-1">
                      {(Object.keys(MSSQL_AUTH_LABELS) as MssqlAuthMethod[]).map((m) => (
                        <button
                          key={m}
                          type="button"
                          onClick={() => set("mssqlAuthMethod", m)}
                          className={cn(
                            "flex-1 rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                            form.mssqlAuthMethod === m
                              ? "bg-primary text-primary-foreground"
                              : "bg-muted text-muted-foreground hover:bg-accent",
                          )}
                        >
                          {MSSQL_AUTH_LABELS[m]}
                        </button>
                      ))}
                    </div>
                  </Field>
                  <label className="flex items-center gap-2 text-xs">
                    <input
                      type="checkbox"
                      checked={form.trustServerCertificate}
                      onChange={(e) => set("trustServerCertificate", e.target.checked)}
                      className="h-3.5 w-3.5 accent-primary"
                    />
                    <span className="text-muted-foreground">Trust Server Certificate</span>
                  </label>
                </>
              )}
              {form.driver === "mssql" && form.mssqlAuthMethod === "entra_id" && (
                <>
                  <Field label="Tenant ID">
                    <input
                      value={form.tenantId}
                      onChange={(e) => set("tenantId", e.target.value)}
                      placeholder="organizations"
                      className="input"
                    />
                  </Field>
                  {deviceCode ? (
                    <div className="rounded-md border border-border bg-muted/50 p-3 text-xs space-y-2">
                      <p className="text-muted-foreground">
                        Go to{" "}
                        <span className="font-medium text-foreground">{deviceCode.verificationUri}</span>
                        {" "}and enter the code:
                      </p>
                      <p className="text-center text-lg font-mono font-bold tracking-widest text-foreground">
                        {deviceCode.userCode}
                      </p>
                      <p className="text-muted-foreground flex items-center gap-1">
                        <Loader2 size={12} className="animate-spin" />
                        Waiting for authentication...
                      </p>
                    </div>
                  ) : (
                    <button
                      type="button"
                      onClick={handleEntraSignIn}
                      disabled={entraLoading}
                      className={cn(
                        "btn-secondary flex items-center gap-2 w-full justify-center",
                        entraSignedIn && "border-success/30 text-success",
                      )}
                    >
                      {entraLoading ? (
                        <Loader2 size={14} className="animate-spin" />
                      ) : entraSignedIn ? (
                        <CheckCircle2 size={14} />
                      ) : (
                        <ExternalLink size={14} />
                      )}
                      {entraSignedIn ? "Signed In" : "Sign in with Microsoft"}
                    </button>
                  )}
                </>
              )}
            </>
          )}

          {/* SQLite file path */}
          {form.driver === "sqlite" && (
            <Field label="Database File">
              <input
                value={form.filePath}
                onChange={(e) => set("filePath", e.target.value)}
                placeholder="/path/to/database.db"
                className="input"
              />
            </Field>
          )}

          {/* DbService backend fields */}
          {isDbService && (
            <>
              <Field label="Service URL">
                <input
                  value={form.dbserviceUrl}
                  onChange={(e) => set("dbserviceUrl", e.target.value)}
                  placeholder="http://localhost:5100"
                  className="input"
                />
              </Field>
              <Field label="API Key">
                <input
                  type="password"
                  value={form.dbserviceApiKey}
                  onChange={(e) => set("dbserviceApiKey", e.target.value)}
                  placeholder="Shared API key (exchanged for a JWT)"
                  className="input"
                />
              </Field>
              <Field label="Remote Connection ID">
                <input
                  value={form.dbserviceRemoteId}
                  onChange={(e) => set("dbserviceRemoteId", e.target.value)}
                  placeholder="ID of a saved connection in DbService"
                  className="input"
                />
              </Field>
              <p className="text-[11px] text-muted-foreground">
                Sqail exchanges the API key at <code>/api/auth/token</code> for a JWT and sends it
                as <code>Authorization: Bearer …</code> on all subsequent requests.
              </p>
            </>
          )}
            </>
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

// ── Database combobox ─────────────────────────────────────

interface DatabaseComboboxProps {
  value: string;
  onChange: (value: string) => void;
  driver: Driver;
  host: string;
  port: number;
  user: string;
  password: string;
  sslMode: string;
  placeholder?: string;
}

function DatabaseCombobox({
  value,
  onChange,
  driver,
  host,
  port,
  user,
  password,
  sslMode,
  placeholder,
}: DatabaseComboboxProps) {
  const [databases, setDatabases] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  const [fetched, setFetched] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Can we fetch? Need at least host, port, user
  const canFetch = !!(host && port && user);

  const fetchDatabases = useCallback(async () => {
    if (!canFetch || loading) return;
    setLoading(true);
    try {
      const dbs = await invoke<string[]>("list_databases", {
        host,
        port,
        user,
        password,
        driver,
        sslMode,
      });
      setDatabases(dbs);
      setFetched(true);
    } catch {
      setDatabases([]);
      setFetched(true);
    } finally {
      setLoading(false);
    }
  }, [canFetch, host, port, user, password, driver, sslMode, loading]);

  // Reset fetched state when connection params change
  useEffect(() => {
    setFetched(false);
    setDatabases([]);
  }, [host, port, user, password, driver, sslMode]);

  const handleOpen = () => {
    if (!canFetch) return;
    if (!fetched) {
      fetchDatabases();
    }
    setOpen(true);
  };

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, [open]);

  // Filter databases by current input
  const filtered = value
    ? databases.filter((db) => db.toLowerCase().includes(value.toLowerCase()))
    : databases;

  return (
    <div ref={containerRef} className="relative">
      <div className="relative">
        <input
          value={value}
          onChange={(e) => {
            onChange(e.target.value);
            if (!open && fetched && databases.length > 0) setOpen(true);
          }}
          onFocus={() => {
            if (fetched && databases.length > 0) setOpen(true);
          }}
          placeholder={placeholder}
          className="input pr-8"
        />
        <button
          type="button"
          onClick={handleOpen}
          disabled={!canFetch}
          className={cn(
            "absolute right-0 top-0 flex h-full items-center px-2 text-muted-foreground",
            canFetch ? "hover:text-foreground" : "opacity-30",
          )}
          title={canFetch ? "Browse databases" : "Enter host, port, and user first"}
        >
          {loading ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <ChevronDown size={12} />
          )}
        </button>
      </div>

      {open && filtered.length > 0 && (
        <div className="absolute z-50 mt-1 w-full max-h-48 overflow-y-auto rounded-md border border-border bg-background py-1 shadow-lg">
          {filtered.map((db) => (
            <button
              key={db}
              type="button"
              onClick={() => {
                onChange(db);
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-center px-3 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground",
                db === value && "bg-primary/10 text-primary font-medium",
              )}
            >
              {db}
            </button>
          ))}
        </div>
      )}

      {open && fetched && filtered.length === 0 && !loading && (
        <div className="absolute z-50 mt-1 w-full rounded-md border border-border bg-background py-2 px-3 shadow-lg text-xs text-muted-foreground">
          {databases.length === 0 ? "Could not retrieve databases" : "No matches"}
        </div>
      )}
    </div>
  );
}

// ── Field helper ──────────────────────────────────────────

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
