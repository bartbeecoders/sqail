export interface ColumnMetadata {
  name: string;
  description: string;
}

export interface GeneratedMetadata {
  description: string;
  columns: ColumnMetadata[];
  exampleUsage: string;
  relatedObjects: string[];
  dependencies: string[];
}

export interface ObjectMetadata {
  id: string;
  connectionId: string;
  schemaName: string;
  objectName: string;
  objectType: "table" | "view" | "function" | "procedure";
  metadata: GeneratedMetadata;
  generatedAt: string;
  updatedAt: string;
}

export interface MetadataProgress {
  connectionId: string;
  current: number;
  total: number;
  objectName: string;
  status: "generating" | "complete" | "error";
}

export interface MetadataDone {
  connectionId: string;
  totalGenerated: number;
}

export interface MetadataError {
  connectionId: string;
  error: string;
}
