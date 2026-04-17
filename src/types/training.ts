export type TrainingPhase =
  | "queued"
  | "preparingDataset"
  | "training"
  | "saving"
  | "completed"
  | "failed"
  | "cancelled";

export interface DatasetOptions {
  schemas: string[];
  sampleRows: number;
  maxTables: number;
  includeMetadata: boolean;
  includeSamples: boolean;
  includeJoins: boolean;
}

export const DEFAULT_DATASET_OPTIONS: DatasetOptions = {
  schemas: [],
  sampleRows: 5,
  maxTables: 200,
  includeMetadata: true,
  includeSamples: true,
  includeJoins: true,
};

export interface TrainingHyperparams {
  epochs: number;
  learningRate: number;
  loraRank: number;
  loraAlpha: number;
  maxSteps: number;
  batchSize: number;
}

export const DEFAULT_HYPERPARAMS: TrainingHyperparams = {
  epochs: 3,
  learningRate: 2e-4,
  loraRank: 8,
  loraAlpha: 16,
  maxSteps: -1,
  batchSize: 1,
};

export interface DatasetStats {
  tableCount: number;
  exampleCount: number;
  filePath: string;
  sizeBytes: number;
}

export interface TrainingJob {
  id: string;
  connectionId: string;
  connectionName: string;
  baseModelId: string;
  baseModelHfId: string;
  phase: TrainingPhase;
  progress: number;
  step: number | null;
  totalSteps: number | null;
  loss: number | null;
  message: string | null;
  error: string | null;
  datasetStats: DatasetStats | null;
  hyperparams: TrainingHyperparams;
  startedAt: string;
  finishedAt: string | null;
  outputModelId: string | null;
  /** Absolute path of the on-disk `trainer.log` for this job. */
  logPath: string | null;
}

export interface TrainedModel {
  id: string;
  displayName: string;
  baseModelId: string;
  connectionId: string;
  connectionName: string;
  datasetSize: number;
  exampleCount: number;
  tableCount: number;
  createdAt: string;
  adapterPath: string;
  ggufPath: string | null;
}

export interface EnvCheck {
  pythonPath: string | null;
  pythonVersion: string | null;
  torchAvailable: boolean;
  transformersAvailable: boolean;
  peftAvailable: boolean;
  trlAvailable: boolean;
  datasetsAvailable: boolean;
  cudaAvailable: boolean;
  cudaDevice: string | null;
  missing: string[];
}

export interface TrainingLogLine {
  id: string;
  line: string;
}
