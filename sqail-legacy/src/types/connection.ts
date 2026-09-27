export type Driver = "postgres" | "mysql" | "sqlite" | "mssql" | "dbservice" | "surrealdb";
export type MssqlAuthMethod = "sql_server" | "windows" | "entra_id";
export type MssqlEncryption = "required" | "login_only" | "off";
/** libpq-style sslmode values. Empty string means "not set" (driver default: prefer). */
export type PgSslMode = "" | "disable" | "allow" | "prefer" | "require" | "verify-ca" | "verify-full";

export interface ConnectionConfig {
  id: string;
  name: string;
  driver: Driver;
  host: string;
  port: number;
  database: string;
  user: string;
  password: string;
  filePath: string;
  sslMode: string;
  /** Postgres: path to the CA certificate (PEM) used to verify the server. */
  sslRootCert: string;
  /** Postgres: path to the client certificate (PEM) for mutual TLS. */
  sslClientCert: string;
  /** Postgres: path to the client private key (unencrypted PEM: PKCS#8, PKCS#1 or SEC1). */
  sslClientKey: string;
  integratedSecurity: boolean;
  trustServerCertificate: boolean;
  mssqlAuthMethod: MssqlAuthMethod;
  mssqlEncryption: MssqlEncryption;
  tenantId: string;
  azureClientId: string;
  color: string;
  // DbService backend
  dbserviceUrl: string;
  dbserviceApiKey: string;
  dbserviceRemoteId: string;
  // SurrealDB
  surrealNamespace: string;
}

export function defaultPort(driver: Driver): number {
  switch (driver) {
    case "postgres":
      return 5432;
    case "mysql":
      return 3306;
    case "mssql":
      return 1433;
    case "sqlite":
      return 0;
    case "dbservice":
      return 0;
    case "surrealdb":
      return 8000;
  }
}

export function defaultConnection(driver: Driver = "postgres"): ConnectionConfig {
  return {
    id: "",
    name: "",
    driver,
    host: "localhost",
    port: defaultPort(driver),
    database: "",
    user: "",
    password: "",
    filePath: "",
    sslMode: "",
    sslRootCert: "",
    sslClientCert: "",
    sslClientKey: "",
    integratedSecurity: false,
    trustServerCertificate: false,
    mssqlAuthMethod: "sql_server",
    mssqlEncryption: "required",
    tenantId: "",
    azureClientId: "",
    color: "",
    dbserviceUrl: "",
    dbserviceApiKey: "",
    dbserviceRemoteId: "",
    surrealNamespace: "",
  };
}

export const DRIVER_LABELS: Record<Driver, string> = {
  postgres: "PostgreSQL",
  mysql: "MySQL",
  sqlite: "SQLite",
  mssql: "SQL Server",
  dbservice: "DbService",
  surrealdb: "SurrealDB",
};

export const MSSQL_AUTH_LABELS: Record<MssqlAuthMethod, string> = {
  sql_server: "SQL Server",
  windows: "Windows",
  entra_id: "Entra ID",
};

export const MSSQL_ENCRYPTION_LABELS: Record<MssqlEncryption, string> = {
  required: "Required",
  login_only: "Login only",
  off: "Off",
};

export const PG_SSL_MODE_LABELS: Record<PgSslMode, string> = {
  "": "Default (prefer)",
  disable: "Disable",
  allow: "Allow",
  prefer: "Prefer",
  require: "Require",
  "verify-ca": "Verify CA",
  "verify-full": "Verify full",
};

const PG_SSL_MODES = new Set<string>(Object.keys(PG_SSL_MODE_LABELS));

/** Narrow an arbitrary string (e.g. from a URL query) to a known sslmode, else "". */
export function toPgSslMode(raw: string | undefined | null): PgSslMode {
  const v = (raw ?? "").toLowerCase();
  return PG_SSL_MODES.has(v) ? (v as PgSslMode) : "";
}

/** Parse the `?a=b&c=d` tail of a URL-style connection string. */
function parseQuery(s: string): Map<string, string> {
  const q = new Map<string, string>();
  const idx = s.indexOf("?");
  if (idx === -1) return q;
  for (const part of s.slice(idx + 1).split("&")) {
    if (!part) continue;
    const eq = part.indexOf("=");
    const key = decodeURIComponent(eq === -1 ? part : part.slice(0, eq)).toLowerCase();
    const val = eq === -1 ? "" : decodeURIComponent(part.slice(eq + 1));
    q.set(key, val);
  }
  return q;
}

/** Parse a connection string into a partial ConnectionConfig. */
export function parseConnectionString(raw: string): Partial<ConnectionConfig> & { driver: Driver } {
  const s = raw.trim();

  // PostgreSQL: postgresql://user:pass@host:port/db  or  postgres://...
  const pgMatch = s.match(/^(?:postgres(?:ql)?):\/\/(?:([^:@]+)(?::([^@]*))?@)?([^:/]+)(?::(\d+))?(?:\/([^?]*))?/i);
  if (pgMatch) {
    // libpq query params: sslmode, sslrootcert, sslcert, sslkey
    const q = parseQuery(s);
    return {
      driver: "postgres",
      user: decodeURIComponent(pgMatch[1] ?? ""),
      password: decodeURIComponent(pgMatch[2] ?? ""),
      host: pgMatch[3] ?? "localhost",
      port: pgMatch[4] ? Number(pgMatch[4]) : 5432,
      database: decodeURIComponent(pgMatch[5] ?? ""),
      sslMode: toPgSslMode(q.get("sslmode")),
      sslRootCert: q.get("sslrootcert") ?? "",
      sslClientCert: q.get("sslcert") ?? "",
      sslClientKey: q.get("sslkey") ?? "",
    };
  }

  // MySQL: mysql://user:pass@host:port/db
  const myMatch = s.match(/^mysql:\/\/(?:([^:@]+)(?::([^@]*))?@)?([^:/]+)(?::(\d+))?(?:\/([^?]*))?/i);
  if (myMatch) {
    return {
      driver: "mysql",
      user: decodeURIComponent(myMatch[1] ?? ""),
      password: decodeURIComponent(myMatch[2] ?? ""),
      host: myMatch[3] ?? "localhost",
      port: myMatch[4] ? Number(myMatch[4]) : 3306,
      database: decodeURIComponent(myMatch[5] ?? ""),
    };
  }

  // SurrealDB: surrealdb://user:pass@host:port/namespace/database
  // or surrealdb+http://...   surrealdb+https://...
  const sdMatch = s.match(
    /^surrealdb(?:\+(https?))?:\/\/(?:([^:@]+)(?::([^@]*))?@)?([^:/]+)(?::(\d+))?(?:\/([^/]+))?(?:\/([^?]*))?/i,
  );
  if (sdMatch) {
    return {
      driver: "surrealdb",
      user: decodeURIComponent(sdMatch[2] ?? ""),
      password: decodeURIComponent(sdMatch[3] ?? ""),
      host: sdMatch[4] ?? "localhost",
      port: sdMatch[5] ? Number(sdMatch[5]) : 8000,
      surrealNamespace: decodeURIComponent(sdMatch[6] ?? ""),
      database: decodeURIComponent(sdMatch[7] ?? ""),
      sslMode: sdMatch[1] === "https" ? "https" : "",
    };
  }

  // SQLite: sqlite://path  or  sqlite:path
  const slMatch = s.match(/^sqlite:(?:\/\/)?(.+)/i);
  if (slMatch) {
    return {
      driver: "sqlite",
      filePath: slMatch[1],
      host: "",
      port: 0,
    };
  }

  // SQL Server key=value format: Server=...;Database=...;User Id=...;Password=...;
  if (/server\s*=/i.test(s) || /data source\s*=/i.test(s)) {
    const kv = new Map<string, string>();
    for (const part of s.split(";")) {
      const eq = part.indexOf("=");
      if (eq === -1) continue;
      const key = part.slice(0, eq).trim().toLowerCase();
      const val = part.slice(eq + 1).trim();
      kv.set(key, val);
    }
    const serverRaw = kv.get("server") ?? kv.get("data source") ?? "localhost";
    let host = serverRaw;
    let port = 1433;
    // Handle server,port or server:port (non-standard but common)
    const portSep = serverRaw.match(/^(.+)[,:](\d+)$/);
    if (portSep) {
      host = portSep[1];
      port = Number(portSep[2]);
    }
    return {
      driver: "mssql",
      host,
      port,
      database: kv.get("database") ?? kv.get("initial catalog") ?? "",
      user: kv.get("user id") ?? kv.get("uid") ?? "",
      password: kv.get("password") ?? kv.get("pwd") ?? "",
      trustServerCertificate: (kv.get("trustservercertificate") ?? "").toLowerCase() === "true",
      integratedSecurity: (kv.get("integrated security") ?? "").toLowerCase() === "true"
        || (kv.get("trusted_connection") ?? "").toLowerCase() === "true",
      // Encrypt=false/optional/no/0 → no encryption (older-server workaround); anything else keeps the default.
      ...(["false", "optional", "no", "0"].includes((kv.get("encrypt") ?? "").toLowerCase())
        ? { mssqlEncryption: "off" as MssqlEncryption }
        : {}),
    };
  }

  // Fallback: unknown format
  throw new Error("Unrecognized connection string format");
}

/** Build a connection string from a ConnectionConfig. */
export function toConnectionString(c: ConnectionConfig): string {
  switch (c.driver) {
    case "postgres": {
      const auth = c.user ? `${encodeURIComponent(c.user)}${c.password ? ":" + encodeURIComponent(c.password) : ""}@` : "";
      const params: string[] = [];
      const mode = toPgSslMode(c.sslMode);
      if (mode) params.push(`sslmode=${mode}`);
      if (c.sslRootCert) params.push(`sslrootcert=${encodeURIComponent(c.sslRootCert)}`);
      if (c.sslClientCert) params.push(`sslcert=${encodeURIComponent(c.sslClientCert)}`);
      if (c.sslClientKey) params.push(`sslkey=${encodeURIComponent(c.sslClientKey)}`);
      const query = params.length ? "?" + params.join("&") : "";
      return `postgresql://${auth}${c.host}:${c.port}/${encodeURIComponent(c.database)}${query}`;
    }
    case "mysql": {
      const auth = c.user ? `${encodeURIComponent(c.user)}${c.password ? ":" + encodeURIComponent(c.password) : ""}@` : "";
      return `mysql://${auth}${c.host}:${c.port}/${encodeURIComponent(c.database)}`;
    }
    case "sqlite":
      return `sqlite://${c.filePath}`;
    case "dbservice":
      return c.dbserviceUrl;
    case "surrealdb": {
      const proto = c.sslMode === "https" ? "surrealdb+https" : "surrealdb";
      const auth = c.user ? `${encodeURIComponent(c.user)}${c.password ? ":" + encodeURIComponent(c.password) : ""}@` : "";
      const ns = c.surrealNamespace ? "/" + encodeURIComponent(c.surrealNamespace) : "";
      const db = c.database ? "/" + encodeURIComponent(c.database) : "";
      return `${proto}://${auth}${c.host}:${c.port}${ns}${db}`;
    }
    case "mssql": {
      const parts: string[] = [];
      parts.push(`Server=${c.host}${c.port !== 1433 ? "," + c.port : ""}`);
      if (c.database) parts.push(`Database=${c.database}`);
      if (c.integratedSecurity) {
        parts.push("Integrated Security=true");
      } else {
        if (c.user) parts.push(`User Id=${c.user}`);
        if (c.password) parts.push(`Password=${c.password}`);
      }
      if (c.trustServerCertificate) parts.push("TrustServerCertificate=true");
      if (c.mssqlEncryption === "off") parts.push("Encrypt=false");
      return parts.join(";") + ";";
    }
  }
}
