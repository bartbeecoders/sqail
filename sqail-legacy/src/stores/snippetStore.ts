import { create } from "zustand";

const STORAGE_KEY = "sqail_snippets";

export interface SqlSnippet {
  id: string;
  prefix: string;      // trigger text (e.g., "sel", "ins")
  name: string;        // display name
  body: string;        // snippet body with $1, $2 placeholders
  description?: string;
}

const BUILTIN_SNIPPETS: SqlSnippet[] = [
  {
    id: "builtin-select",
    prefix: "sel",
    name: "SELECT",
    body: "SELECT ${1:*}\nFROM ${2:table}\nWHERE ${3:1=1};",
    description: "Basic SELECT statement",
  },
  {
    id: "builtin-select-top",
    prefix: "selt",
    name: "SELECT TOP/LIMIT",
    body: "SELECT ${1:*}\nFROM ${2:table}\nLIMIT ${3:100};",
    description: "SELECT with LIMIT",
  },
  {
    id: "builtin-insert",
    prefix: "ins",
    name: "INSERT INTO",
    body: "INSERT INTO ${1:table} (${2:columns})\nVALUES (${3:values});",
    description: "INSERT statement",
  },
  {
    id: "builtin-update",
    prefix: "upd",
    name: "UPDATE",
    body: "UPDATE ${1:table}\nSET ${2:column} = ${3:value}\nWHERE ${4:condition};",
    description: "UPDATE statement",
  },
  {
    id: "builtin-delete",
    prefix: "del",
    name: "DELETE",
    body: "DELETE FROM ${1:table}\nWHERE ${2:condition};",
    description: "DELETE statement",
  },
  {
    id: "builtin-create-table",
    prefix: "ct",
    name: "CREATE TABLE",
    body: "CREATE TABLE ${1:table_name} (\n  ${2:id} SERIAL PRIMARY KEY,\n  ${3:column} ${4:VARCHAR(255)} NOT NULL\n);",
    description: "CREATE TABLE template",
  },
  {
    id: "builtin-join",
    prefix: "ij",
    name: "INNER JOIN",
    body: "INNER JOIN ${1:table} ON ${2:condition}",
    description: "INNER JOIN clause",
  },
  {
    id: "builtin-left-join",
    prefix: "lj",
    name: "LEFT JOIN",
    body: "LEFT JOIN ${1:table} ON ${2:condition}",
    description: "LEFT JOIN clause",
  },
  {
    id: "builtin-cte",
    prefix: "cte",
    name: "CTE (WITH)",
    body: "WITH ${1:cte_name} AS (\n  ${2:SELECT 1}\n)\nSELECT ${3:*}\nFROM ${1:cte_name};",
    description: "Common Table Expression",
  },
  {
    id: "builtin-case",
    prefix: "case",
    name: "CASE WHEN",
    body: "CASE\n  WHEN ${1:condition} THEN ${2:result}\n  ELSE ${3:default}\nEND",
    description: "CASE expression",
  },
  {
    id: "builtin-index",
    prefix: "ci",
    name: "CREATE INDEX",
    body: "CREATE INDEX ${1:idx_name} ON ${2:table} (${3:column});",
    description: "CREATE INDEX",
  },
  {
    id: "builtin-alter-add",
    prefix: "aa",
    name: "ALTER TABLE ADD",
    body: "ALTER TABLE ${1:table}\nADD COLUMN ${2:column} ${3:VARCHAR(255)};",
    description: "Add column to table",
  },
];

function loadSnippets(): SqlSnippet[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw);
  } catch { /* ignore */ }
  return [];
}

interface SnippetState {
  userSnippets: SqlSnippet[];
  allSnippets: SqlSnippet[];

  addSnippet: (snippet: SqlSnippet) => void;
  updateSnippet: (snippet: SqlSnippet) => void;
  deleteSnippet: (id: string) => void;
  resetToDefaults: () => void;
}

export const useSnippetStore = create<SnippetState>((set, get) => {
  const userSnippets = loadSnippets();

  const persist = () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(get().userSnippets));
  };

  const computeAll = (user: SqlSnippet[]) => {
    // User snippets override builtins with same prefix
    const userPrefixes = new Set(user.map((s) => s.prefix));
    const builtins = BUILTIN_SNIPPETS.filter((s) => !userPrefixes.has(s.prefix));
    return [...builtins, ...user];
  };

  return {
    userSnippets,
    allSnippets: computeAll(userSnippets),

    addSnippet: (snippet) => {
      const user = [...get().userSnippets, snippet];
      set({ userSnippets: user, allSnippets: computeAll(user) });
      persist();
    },

    updateSnippet: (snippet) => {
      const user = get().userSnippets.map((s) => (s.id === snippet.id ? snippet : s));
      set({ userSnippets: user, allSnippets: computeAll(user) });
      persist();
    },

    deleteSnippet: (id) => {
      const user = get().userSnippets.filter((s) => s.id !== id);
      set({ userSnippets: user, allSnippets: computeAll(user) });
      persist();
    },

    resetToDefaults: () => {
      set({ userSnippets: [], allSnippets: computeAll([]) });
      localStorage.removeItem(STORAGE_KEY);
    },
  };
});
