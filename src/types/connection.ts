export type Driver = "postgres" | "mysql" | "sqlite" | "mssql";

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
    color: "",
  };
}

export const DRIVER_LABELS: Record<Driver, string> = {
  postgres: "PostgreSQL",
  mysql: "MySQL",
  sqlite: "SQLite",
  mssql: "SQL Server",
};
