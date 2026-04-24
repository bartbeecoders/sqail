import { invoke } from "@tauri-apps/api/core";
import type { ConnectionConfig } from "../../types/connection";
import {
  SQAIL_FILE_VERSION,
  type EncryptedField,
  type SqailConnection,
  type SqailDiagramPayload,
  type SqailFile,
  type SqailPayload,
  type SqailPlainPayload,
  type SqailProjectFile,
  type SqailSqlPayload,
} from "../../types/sqailFile";

/** Build a `.sqail` file envelope around a payload. */
export async function encodeSqailFile(
  payload: SqailPlainPayload,
  passphrase?: string,
): Promise<SqailFile> {
  // Scan the payload for embedded connections and encrypt their sensitive fields.
  // The encoded form is immutable from the caller's point of view — we clone.
  const encoded = await encryptPayloadSecrets(payload, passphrase);
  const now = new Date().toISOString();
  return {
    magic: "sqail",
    version: SQAIL_FILE_VERSION,
    createdAt: now,
    updatedAt: now,
    encrypted: encoded.hadSecrets,
    passphraseProtected: encoded.hadSecrets && !!passphrase,
    payload: encoded.payload,
  };
}

/** Serialise a `.sqail` file envelope to pretty JSON. */
export function serializeSqailFile(file: SqailFile): string {
  return JSON.stringify(file, null, 2);
}

/** Parse a `.sqail` file envelope from JSON. Throws on magic/version mismatch. */
export function parseSqailFile(raw: string): SqailFile {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error("Not a valid JSON file");
  }
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    (parsed as { magic?: unknown }).magic !== "sqail"
  ) {
    throw new Error("Not a sqail file (missing magic)");
  }
  const file = parsed as SqailFile;
  if (file.version > SQAIL_FILE_VERSION) {
    throw new Error(
      `Unsupported .sqail version ${file.version} — this build supports up to v${SQAIL_FILE_VERSION}`,
    );
  }
  return file;
}

/** Decrypt sensitive fields inside a decoded `.sqail`. Returns the plain payload. */
export async function decryptPayloadSecrets(
  payload: SqailPayload,
  passphrase?: string,
): Promise<SqailPlainPayload> {
  switch (payload.kind) {
    case "sql": {
      const p = payload.data;
      return {
        kind: "sql",
        data: {
          title: p.title,
          sql: p.sql,
          promptHistory: p.promptHistory,
          connection: p.connection ? await decryptConnection(p.connection, passphrase) : undefined,
        },
      };
    }
    case "diagram": {
      const p = payload.data;
      return {
        kind: "diagram",
        data: {
          title: p.title,
          diagram: p.diagram,
          promptHistory: p.promptHistory,
          connection: p.connection ? await decryptConnection(p.connection, passphrase) : undefined,
        },
      };
    }
    case "project": {
      const src = payload.data;
      const connections = src.connections
        ? await Promise.all(src.connections.map((c) => decryptConnection(c, passphrase)))
        : undefined;
      const files: SqailProjectFile<ConnectionConfig>[] = await Promise.all(
        src.files.map(async (f): Promise<SqailProjectFile<ConnectionConfig>> => {
          if (f.kind === "sql") {
            const fp = f.payload as SqailSqlPayload;
            const payload: SqailSqlPayload<ConnectionConfig> = {
              title: fp.title,
              sql: fp.sql,
              promptHistory: fp.promptHistory,
              connection: fp.connection
                ? await decryptConnection(fp.connection, passphrase)
                : undefined,
            };
            return { kind: "sql", payload };
          }
          const fp = f.payload as SqailDiagramPayload;
          const payload: SqailDiagramPayload<ConnectionConfig> = {
            title: fp.title,
            diagram: fp.diagram,
            promptHistory: fp.promptHistory,
            connection: fp.connection
              ? await decryptConnection(fp.connection, passphrase)
              : undefined,
          };
          return { kind: "diagram", payload };
        }),
      );
      return { kind: "project", data: { name: src.name, files, connections } };
    }
  }
}

/** Strip encryption envelopes from a `SqailConnection` back into a usable `ConnectionConfig`. */
export async function decryptConnection(
  encrypted: SqailConnection,
  passphrase?: string,
): Promise<ConnectionConfig> {
  const password = await decryptMaybe(encrypted.password, passphrase);
  const dbserviceApiKey = await decryptMaybe(encrypted.dbserviceApiKey, passphrase);
  return { ...encrypted, password, dbserviceApiKey };
}

// ---------------------------------------------------------------------------

async function encryptPayloadSecrets(
  payload: SqailPlainPayload,
  passphrase?: string,
): Promise<{ payload: SqailPayload; hadSecrets: boolean }> {
  let hadSecrets = false;
  const encryptConn = async (c: ConnectionConfig): Promise<SqailConnection> => {
    const password = c.password ? await encryptString(c.password, passphrase) : "";
    const dbserviceApiKey = c.dbserviceApiKey
      ? await encryptString(c.dbserviceApiKey, passphrase)
      : "";
    if (password !== "" || dbserviceApiKey !== "") hadSecrets = true;
    return { ...c, password, dbserviceApiKey };
  };

  switch (payload.kind) {
    case "sql": {
      const data = { ...payload.data };
      const out: SqailPayload = {
        kind: "sql",
        data: {
          ...data,
          connection: data.connection ? await encryptConn(data.connection) : undefined,
        },
      };
      return { payload: out, hadSecrets };
    }
    case "diagram": {
      const data = { ...payload.data };
      const out: SqailPayload = {
        kind: "diagram",
        data: {
          ...data,
          connection: data.connection ? await encryptConn(data.connection) : undefined,
        },
      };
      return { payload: out, hadSecrets };
    }
    case "project": {
      const src = payload.data;
      const connections = src.connections
        ? await Promise.all(src.connections.map((c) => encryptConn(c)))
        : undefined;
      const files: SqailProjectFile[] = await Promise.all(
        src.files.map(async (f): Promise<SqailProjectFile> => {
          if (f.kind === "sql") {
            const p = f.payload as SqailSqlPayload<ConnectionConfig>;
            const payload: SqailSqlPayload = {
              title: p.title,
              sql: p.sql,
              promptHistory: p.promptHistory,
              connection: p.connection ? await encryptConn(p.connection) : undefined,
            };
            return { kind: "sql", payload };
          }
          const p = f.payload as SqailDiagramPayload<ConnectionConfig>;
          const payload: SqailDiagramPayload = {
            title: p.title,
            diagram: p.diagram,
            promptHistory: p.promptHistory,
            connection: p.connection ? await encryptConn(p.connection) : undefined,
          };
          return { kind: "diagram", payload };
        }),
      );
      return {
        payload: { kind: "project", data: { name: src.name, files, connections } },
        hadSecrets,
      };
    }
  }
}

async function encryptString(
  plaintext: string,
  passphrase: string | undefined,
): Promise<EncryptedField> {
  return invoke<EncryptedField>("sqail_encrypt_secret", {
    plaintext,
    passphrase: passphrase ?? null,
  });
}

async function decryptMaybe(
  field: EncryptedField | "",
  passphrase: string | undefined,
): Promise<string> {
  if (field === "") return "";
  return invoke<string>("sqail_decrypt_secret", {
    envelope: field,
    passphrase: passphrase ?? null,
  });
}
