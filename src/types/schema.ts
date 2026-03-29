export interface SchemaInfo {
  name: string;
}

export interface TableInfo {
  name: string;
  schema: string;
  tableType: "table" | "view";
}

export interface ColumnInfo {
  name: string;
  dataType: string;
  isNullable: boolean;
  columnDefault: string | null;
  isPrimaryKey: boolean;
  ordinalPosition: number;
}

export interface IndexInfo {
  name: string;
  isUnique: boolean;
  columns: string[];
}

export interface RoutineInfo {
  name: string;
  schema: string;
  routineType: "function" | "procedure";
}
