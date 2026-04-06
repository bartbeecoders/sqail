export type Driver = "postgres" | "mysql" | "sqlite" | "mssql";
export type MssqlAuthMethod = "sql_server" | "windows" | "entra_id";

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
  integratedSecurity: boolean;
  trustServerCertificate: boolean;
  mssqlAuthMethod: MssqlAuthMethod;
  tenantId: string;
  azureClientId: string;
  color: string;
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
    integratedSecurity: false,
    trustServerCertificate: false,
    mssqlAuthMethod: "sql_server",
    tenantId: "",
    azureClientId: "",
    color: "",
  };
}

export const DRIVER_LABELS: Record<Driver, string> = {
  postgres: "PostgreSQL",
  mysql: "MySQL",
  sqlite: "SQLite",
  mssql: "SQL Server",
};

export const MSSQL_AUTH_LABELS: Record<MssqlAuthMethod, string> = {
  sql_server: "SQL Server",
  windows: "Windows",
  entra_id: "Entra ID",
};

/** Parse a connection string into a partial ConnectionConfig. */
export function parseConnectionString(raw: string): Partial<ConnectionConfig> & { driver: Driver } {
  const s = raw.trim();

  // PostgreSQL: postgresql://user:pass@host:port/db  or  postgres://...
  const pgMatch = s.match(/^(?:postgres(?:ql)?):\/\/(?:([^:@]+)(?::([^@]*))?@)?([^:/]+)(?::(\d+))?(?:\/([^?]*))?/i);
  if (pgMatch) {
    return {
      driver: "postgres",
      user: decodeURIComponent(pgMatch[1] ?? ""),
      password: decodeURIComponent(pgMatch[2] ?? ""),
      host: pgMatch[3] ?? "localhost",
      port: pgMatch[4] ? Number(pgMatch[4]) : 5432,
      database: decodeURIComponent(pgMatch[5] ?? ""),
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
      return `postgresql://${auth}${c.host}:${c.port}/${encodeURIComponent(c.database)}`;
    }
    case "mysql": {
      const auth = c.user ? `${encodeURIComponent(c.user)}${c.password ? ":" + encodeURIComponent(c.password) : ""}@` : "";
      return `mysql://${auth}${c.host}:${c.port}/${encodeURIComponent(c.database)}`;
    }
    case "sqlite":
      return `sqlite://${c.filePath}`;
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
      return parts.join(";") + ";";
    }
  }
}
