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
