export interface DiagramTablePosition {
  key: string; // `${schema}.${table}`
  x: number;
  y: number;
  color?: string; // header/border color override
}

export type AnnotationShape = "rect" | "text" | "line";

export interface DiagramAnnotation {
  id: string;
  shape: AnnotationShape;
  x: number;
  y: number;
  // For rect: width/height. For line: relative endpoint (x2, y2 are absolute).
  width?: number;
  height?: number;
  x2?: number;
  y2?: number;
  text?: string;
  color?: string;
  fill?: string;
  fontSize?: number;
  strokeWidth?: number;
}

export interface DiagramColorSettings {
  tableHeader: string;
  tableBorder: string;
  tableBody: string;
  pkColor: string;
  fkColor: string;
  columnText: string;
  relationship: string;
}

export interface DiagramState {
  schemaName: string;
  zoom: number;
  panX: number;
  panY: number;
  positions: Record<string, DiagramTablePosition>;
  annotations: DiagramAnnotation[];
  colors?: Partial<DiagramColorSettings>;
  snapToGrid?: boolean;
  // `true` if tables/FKs have been loaded at least once
  loaded?: boolean;
}

export const DIAGRAM_GRID_SIZE = 20;

export function snapValue(v: number, enabled: boolean | undefined): number {
  return enabled ? Math.round(v / DIAGRAM_GRID_SIZE) * DIAGRAM_GRID_SIZE : v;
}

export function defaultDiagramState(schemaName: string): DiagramState {
  return {
    schemaName,
    zoom: 1,
    panX: 0,
    panY: 0,
    positions: {},
    annotations: [],
    colors: {},
  };
}
