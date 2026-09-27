import type { ConnectionConfig } from "./connection";
import type { DiagramState } from "./diagram";

/**
 * `.sqail` file format (v1).
 *
 * Human-readable JSON. Sensitive strings (connection password, API tokens)
 * are replaced with a {@link EncryptedField} envelope produced by the Rust
 * `sqail_encrypt_secret` command.
 *
 * Three top-level kinds:
 *  - `sql`     — a single SQL editor tab's content + optional connection + per-tab AI prompt history
 *  - `diagram` — a single diagram tab's state + optional connection + per-tab AI prompt history
 *  - `project` — a bundle of files (SQL + diagram payloads) + shared connections
 */

export const SQAIL_FILE_VERSION = 1;

/** Sentinel for an encrypted string field inside the JSON payload. */
export interface EncryptedField {
  $enc: true;
  alg: string; // e.g. "AES-256-GCM/machine" or "AES-256-GCM/argon2id"
  nonce: string; // base64
  ct: string; // base64 ciphertext
  salt?: string; // base64, passphrase mode only
}

/** One entry in a per-file AI prompt history. Separate from the global DB-backed history. */
export interface SqailPromptEntry {
  timestamp: string;
  flow: string;
  prompt: string;
  response: string;
}

/** A connection as stored inside a `.sqail` — password is an encrypted envelope. */
export interface SqailConnection extends Omit<ConnectionConfig, "password" | "dbserviceApiKey"> {
  password: EncryptedField | "";
  dbserviceApiKey: EncryptedField | "";
}

/**
 * Payloads are parameterised over the connection representation:
 *  - `ConnectionConfig` — plain, used when building a file in-memory before encoding.
 *  - `SqailConnection`  — encrypted envelope form, used in the serialised file.
 */

export interface SqailSqlPayload<C = SqailConnection> {
  title: string;
  sql: string;
  connection?: C;
  promptHistory?: SqailPromptEntry[];
}

export interface SqailDiagramPayload<C = SqailConnection> {
  title: string;
  diagram: DiagramState;
  connection?: C;
  promptHistory?: SqailPromptEntry[];
}

export interface SqailProjectFile<C = SqailConnection> {
  kind: "sql" | "diagram";
  payload: SqailSqlPayload<C> | SqailDiagramPayload<C>;
}

export interface SqailProjectGitConfig {
  repoPath: string;
  connectionId?: string;
  defaultRemote?: string;
  includeRoutines?: boolean;
  lastSnapshotAt?: string;
}

export interface SqailProjectPayload<C = SqailConnection> {
  name: string;
  files: SqailProjectFile<C>[];
  connections?: C[];
  git?: SqailProjectGitConfig;
}

export type SqailPayload<C = SqailConnection> =
  | { kind: "sql"; data: SqailSqlPayload<C> }
  | { kind: "diagram"; data: SqailDiagramPayload<C> }
  | { kind: "project"; data: SqailProjectPayload<C> };

/** Pre-encryption payload supplied by callers. Re-exported for convenience. */
export type SqailPlainPayload = SqailPayload<ConnectionConfig>;

export interface SqailFile {
  /** Magic identifier — lets the codec reject unrelated `.json` files. */
  magic: "sqail";
  version: number;
  createdAt: string;
  updatedAt: string;
  /** `true` if the file contains any encrypted fields that need the machine key or a passphrase. */
  encrypted: boolean;
  /** `true` if decryption requires the user to provide a passphrase. */
  passphraseProtected: boolean;
  payload: SqailPayload;
}

export function isEncryptedField(v: unknown): v is EncryptedField {
  return (
    typeof v === "object" && v !== null && (v as { $enc?: unknown }).$enc === true
  );
}
