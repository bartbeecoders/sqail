/**
 * Custom SQL formatter that aligns columns, adds table aliases,
 * and produces PascalCase AS aliases without spaces.
 *
 * Example output:
 *   SELECT
 *       eg.[id]                   AS 'Id',
 *       eg.[plant_cd]             AS 'PlantCd'
 *   FROM [schema].[table] eg
 *   WHERE eg.[active] = 1;
 */

const KEYWORDS = new Set([
  "SELECT",
  "FROM",
  "WHERE",
  "ORDER",
  "GROUP",
  "HAVING",
  "JOIN",
  "INNER",
  "LEFT",
  "RIGHT",
  "OUTER",
  "CROSS",
  "ON",
  "AND",
  "OR",
  "INSERT",
  "INTO",
  "UPDATE",
  "DELETE",
  "SET",
  "VALUES",
  "CREATE",
  "ALTER",
  "DROP",
  "TABLE",
  "INDEX",
  "VIEW",
  "AS",
  "IN",
  "NOT",
  "NULL",
  "IS",
  "LIKE",
  "BETWEEN",
  "EXISTS",
  "CASE",
  "WHEN",
  "THEN",
  "ELSE",
  "END",
  "UNION",
  "ALL",
  "DISTINCT",
  "TOP",
  "LIMIT",
  "OFFSET",
  "ASC",
  "DESC",
  "BY",
  "WITH",
  "OVER",
  "PARTITION",
  "ROWS",
  "RANGE",
  "FETCH",
  "NEXT",
  "ONLY",
  "PERCENT",
  "TIES",
]);

/** Major clause keywords that start on their own line. */
const CLAUSE_STARTS = new Set([
  "SELECT",
  "FROM",
  "WHERE",
  "ORDER BY",
  "GROUP BY",
  "HAVING",
  "INNER JOIN",
  "LEFT JOIN",
  "LEFT OUTER JOIN",
  "RIGHT JOIN",
  "RIGHT OUTER JOIN",
  "CROSS JOIN",
  "FULL JOIN",
  "FULL OUTER JOIN",
  "JOIN",
  "ON",
  "UNION",
  "UNION ALL",
  "INSERT INTO",
  "UPDATE",
  "DELETE FROM",
  "SET",
  "VALUES",
  "WITH",
  "LIMIT",
  "OFFSET",
  "FETCH",
]);

/** Convert snake_case to PascalCase (no spaces). */
function toPascalCase(name: string): string {
  return name
    .replace(/^[[\]"`]+|[[\]"`]+$/g, "") // strip quotes/brackets
    .split(/[_\s-]+/)
    .map((w) => (w.length > 0 ? w[0].toUpperCase() + w.slice(1).toLowerCase() : ""))
    .join("");
}

/** Generate a short table alias from a qualified or unqualified table name.
 *
 * Qualified:   [mas].[equipment_group] → "mg"  (schema initial + last word initial)
 * Unqualified: [equipment_group]       → "eg"  (initials of underscore-separated words)
 * Single word: [users]                 → "us"  (first two chars)
 */
function generateTableAlias(tableName: string): string {
  const clean = tableName.replace(/[[\]"`]/g, "");
  const dotParts = clean.split(".");

  if (dotParts.length >= 2) {
    // Qualified name: schema initial + last word of table name
    const schema = dotParts[dotParts.length - 2];
    const table = dotParts[dotParts.length - 1];
    const tableWords = table.split(/[_\s-]+/);
    const lastWord = tableWords[tableWords.length - 1];
    return (schema[0] + lastWord[0]).toLowerCase();
  }

  // Unqualified: initials of underscore-separated words
  const words = clean.split(/[_\s-]+/);
  if (words.length > 1) {
    return words.map((w) => w[0]?.toLowerCase() ?? "").join("");
  }
  return clean.slice(0, Math.min(2, clean.length)).toLowerCase();
}

interface Token {
  type: "keyword" | "ident" | "string" | "number" | "symbol" | "whitespace" | "comment";
  value: string;
  upper: string;
}

/** Simple SQL tokenizer. */
function tokenize(sql: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;

  while (i < sql.length) {
    // Whitespace
    if (/\s/.test(sql[i])) {
      const start = i;
      while (i < sql.length && /\s/.test(sql[i])) i++;
      tokens.push({ type: "whitespace", value: sql.slice(start, i), upper: " " });
      continue;
    }

    // Single-line comment
    if (sql[i] === "-" && sql[i + 1] === "-") {
      const start = i;
      while (i < sql.length && sql[i] !== "\n") i++;
      tokens.push({ type: "comment", value: sql.slice(start, i), upper: sql.slice(start, i) });
      continue;
    }

    // Multi-line comment
    if (sql[i] === "/" && sql[i + 1] === "*") {
      const start = i;
      i += 2;
      while (i < sql.length - 1 && !(sql[i] === "*" && sql[i + 1] === "/")) i++;
      i += 2;
      tokens.push({ type: "comment", value: sql.slice(start, i), upper: sql.slice(start, i) });
      continue;
    }

    // Bracketed identifier [...]
    if (sql[i] === "[") {
      const start = i;
      i++;
      while (i < sql.length && sql[i] !== "]") i++;
      i++; // closing ]
      tokens.push({ type: "ident", value: sql.slice(start, i), upper: sql.slice(start, i) });
      continue;
    }

    // Quoted identifier "..."
    if (sql[i] === '"') {
      const start = i;
      i++;
      while (i < sql.length && sql[i] !== '"') i++;
      i++;
      tokens.push({ type: "ident", value: sql.slice(start, i), upper: sql.slice(start, i) });
      continue;
    }

    // Backtick identifier `...`
    if (sql[i] === "`") {
      const start = i;
      i++;
      while (i < sql.length && sql[i] !== "`") i++;
      i++;
      tokens.push({ type: "ident", value: sql.slice(start, i), upper: sql.slice(start, i) });
      continue;
    }

    // String literal '...'
    if (sql[i] === "'") {
      const start = i;
      i++;
      while (i < sql.length) {
        if (sql[i] === "'" && sql[i + 1] === "'") {
          i += 2; // escaped quote
        } else if (sql[i] === "'") {
          i++;
          break;
        } else {
          i++;
        }
      }
      tokens.push({ type: "string", value: sql.slice(start, i), upper: sql.slice(start, i) });
      continue;
    }

    // Number
    if (/\d/.test(sql[i])) {
      const start = i;
      while (i < sql.length && /[\d.]/.test(sql[i])) i++;
      tokens.push({ type: "number", value: sql.slice(start, i), upper: sql.slice(start, i) });
      continue;
    }

    // Word (keyword or identifier)
    if (/[a-zA-Z_@#]/.test(sql[i])) {
      const start = i;
      while (i < sql.length && /[a-zA-Z0-9_@#$]/.test(sql[i])) i++;
      const word = sql.slice(start, i);
      const upper = word.toUpperCase();
      const type = KEYWORDS.has(upper) ? "keyword" : "ident";
      tokens.push({ type, value: word, upper });
      continue;
    }

    // Symbols (operators, punctuation)
    tokens.push({ type: "symbol", value: sql[i], upper: sql[i] });
    i++;
  }

  return tokens;
}

/** Filter out whitespace/comment tokens for parsing. */
function meaningful(tokens: Token[]): Token[] {
  return tokens.filter((t) => t.type !== "whitespace" && t.type !== "comment");
}

interface SelectColumn {
  expr: string; // e.g. "t.[column_name]" or "t.[col]"
  alias: string | null; // e.g. "'ColumnName'" or null
  raw: string; // full original text of this column item
}

interface FromTable {
  expr: string; // e.g. "[schema].[table]"
  alias: string | null;
}

interface ParsedSelect {
  distinct: boolean;
  top: string | null;
  columns: SelectColumn[];
  fromTables: FromTable[];
  restClauses: { keyword: string; body: string }[];
  trailingSemicolon: boolean;
}

/** Try to parse a SELECT statement from tokens. Returns null if not a SELECT. */
function parseSelect(tokens: Token[]): ParsedSelect | null {
  const mt = meaningful(tokens);
  if (mt.length === 0 || mt[0].upper !== "SELECT") return null;

  let pos = 1;
  let distinct = false;
  let top: string | null = null;

  // DISTINCT
  if (pos < mt.length && mt[pos].upper === "DISTINCT") {
    distinct = true;
    pos++;
  }

  // TOP N [PERCENT]
  if (pos < mt.length && mt[pos].upper === "TOP") {
    pos++;
    let topExpr = "";
    // Could be TOP (expr) or TOP N
    if (pos < mt.length && mt[pos].value === "(") {
      let depth = 0;
      while (pos < mt.length) {
        if (mt[pos].value === "(") depth++;
        if (mt[pos].value === ")") depth--;
        topExpr += mt[pos].value;
        pos++;
        if (depth === 0) break;
      }
    } else if (pos < mt.length) {
      topExpr = mt[pos].value;
      pos++;
    }
    if (pos < mt.length && mt[pos].upper === "PERCENT") {
      topExpr += " PERCENT";
      pos++;
    }
    if (pos < mt.length && mt[pos].upper === "WITH" && pos + 1 < mt.length && mt[pos + 1].upper === "TIES") {
      topExpr += " WITH TIES";
      pos += 2;
    }
    top = topExpr;
  }

  // Parse SELECT columns until FROM
  const columns: SelectColumn[] = [];
  const colStart = pos;

  // Find FROM position at top level (not inside parens)
  let fromPos = -1;
  let depth = 0;
  for (let i = pos; i < mt.length; i++) {
    if (mt[i].value === "(") depth++;
    if (mt[i].value === ")") depth--;
    if (depth === 0 && mt[i].upper === "FROM") {
      fromPos = i;
      break;
    }
  }

  if (fromPos === -1) {
    // No FROM — treat everything as columns (but stop at semicolon)
    fromPos = mt.length;
    for (let i = pos; i < mt.length; i++) {
      if (mt[i].value === ";") {
        fromPos = i;
        break;
      }
    }
  }

  // Split columns by comma at depth 0
  let currentColTokens: Token[] = [];
  depth = 0;
  for (let i = colStart; i < fromPos; i++) {
    if (mt[i].value === ";") break;
    if (mt[i].value === "(") depth++;
    if (mt[i].value === ")") depth--;
    if (depth === 0 && mt[i].value === ",") {
      columns.push(parseColumnTokens(currentColTokens));
      currentColTokens = [];
    } else {
      currentColTokens.push(mt[i]);
    }
  }
  if (currentColTokens.length > 0) {
    columns.push(parseColumnTokens(currentColTokens));
  }

  // Parse FROM tables
  const fromTables: FromTable[] = [];
  const restClauses: { keyword: string; body: string }[] = [];
  let trailingSemicolon = mt.some((t) => t.value === ";");

  if (fromPos < mt.length && mt[fromPos]?.upper === "FROM") {
    pos = fromPos + 1; // skip FROM

    // Find end of FROM clause (next major keyword or semicolon at depth 0)
    let fromEnd = mt.length;
    depth = 0;
    for (let i = pos; i < mt.length; i++) {
      if (mt[i].value === ";") {
        fromEnd = i;
        break;
      }
      if (mt[i].value === "(") depth++;
      if (mt[i].value === ")") depth--;
      if (depth === 0) {
        const twoWord = i + 1 < mt.length ? mt[i].upper + " " + mt[i + 1].upper : "";
        const kw = mt[i].upper;
        if (
          CLAUSE_STARTS.has(kw) &&
          kw !== "FROM" &&
          kw !== "ON" // ON is part of JOIN
        ) {
          fromEnd = i;
          break;
        }
        if (CLAUSE_STARTS.has(twoWord)) {
          fromEnd = i;
          break;
        }
      }
    }

    // Parse FROM table references (comma-separated)
    let tableTokens: Token[] = [];
    depth = 0;
    for (let i = pos; i < fromEnd; i++) {
      if (mt[i].value === "(") depth++;
      if (mt[i].value === ")") depth--;
      if (depth === 0 && mt[i].value === ",") {
        fromTables.push(parseTableTokens(tableTokens));
        tableTokens = [];
      } else {
        tableTokens.push(mt[i]);
      }
    }
    if (tableTokens.length > 0) {
      fromTables.push(parseTableTokens(tableTokens));
    }

    // Collect rest of the statement as clause blocks
    pos = fromEnd;
    while (pos < mt.length) {
      if (mt[pos].value === ";") {
        trailingSemicolon = true;
        pos++;
        continue;
      }

      // Detect clause keyword (1 or 2 words)
      let keyword: string;
      let skipCount: number;
      const twoWord = pos + 1 < mt.length ? mt[pos].upper + " " + mt[pos + 1].upper : "";
      if (CLAUSE_STARTS.has(twoWord)) {
        keyword = twoWord;
        skipCount = 2;
      } else if (CLAUSE_STARTS.has(mt[pos].upper)) {
        keyword = mt[pos].upper;
        skipCount = 1;
      } else {
        // Not a recognized clause start — collect as body of previous or standalone
        keyword = mt[pos].upper;
        skipCount = 1;
      }

      pos += skipCount;

      // Collect body tokens until next clause start
      const bodyTokens: Token[] = [];
      depth = 0;
      while (pos < mt.length) {
        if (mt[pos].value === "(") depth++;
        if (mt[pos].value === ")") depth--;
        if (mt[pos].value === ";") break;
        if (depth === 0) {
          const tw = pos + 1 < mt.length ? mt[pos].upper + " " + mt[pos + 1].upper : "";
          if (CLAUSE_STARTS.has(tw) || CLAUSE_STARTS.has(mt[pos].upper)) {
            break;
          }
        }
        bodyTokens.push(mt[pos]);
        pos++;
      }

      restClauses.push({ keyword, body: joinTokens(bodyTokens) });
    }
  }

  return { distinct, top, columns, fromTables, restClauses, trailingSemicolon };
}

/** Join tokens with smart spacing: no space around dots. */
function joinTokens(tokens: Token[], uppercaseKeywords = true): string {
  let result = "";
  for (let i = 0; i < tokens.length; i++) {
    const val = uppercaseKeywords && tokens[i].type === "keyword" ? tokens[i].upper : tokens[i].value;
    if (i > 0 && val !== "." && tokens[i - 1].value !== ".") {
      result += " ";
    }
    result += val;
  }
  return result;
}

function parseColumnTokens(tokens: Token[]): SelectColumn {
  // Look for AS keyword
  let asIdx = -1;
  let depth = 0;
  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].value === "(") depth++;
    if (tokens[i].value === ")") depth--;
    if (depth === 0 && tokens[i].upper === "AS") {
      asIdx = i;
      break;
    }
  }

  if (asIdx !== -1 && asIdx + 1 < tokens.length) {
    const exprTokens = tokens.slice(0, asIdx);
    const aliasTokens = tokens.slice(asIdx + 1);
    return {
      expr: joinTokens(exprTokens),
      alias: joinTokens(aliasTokens, false),
      raw: joinTokens(tokens),
    };
  }

  // No AS
  const expr = joinTokens(tokens);
  return { expr, alias: null, raw: expr };
}

function parseTableTokens(tokens: Token[]): FromTable {
  // Look for AS keyword or implicit alias
  let asIdx = -1;
  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].upper === "AS") {
      asIdx = i;
      break;
    }
  }

  if (asIdx !== -1 && asIdx + 1 < tokens.length) {
    const alias = tokens[asIdx + 1].value;
    return { expr: normalizeTableExpr(tokens.slice(0, asIdx)), alias };
  }

  // Check for implicit alias: [schema].[table] alias
  // The last token could be an implicit alias if it's an ident without dots before it
  if (tokens.length >= 2) {
    const last = tokens[tokens.length - 1];
    const beforeLast = tokens[tokens.length - 2];
    if (last.type === "ident" && beforeLast.value !== ".") {
      return {
        expr: normalizeTableExpr(tokens.slice(0, -1)),
        alias: last.value,
      };
    }
  }

  return { expr: normalizeTableExpr(tokens), alias: null };
}

function normalizeTableExpr(tokens: Token[]): string {
  return tokens
    .map((t) => t.value)
    .join("")
    .replace(/\.\./g, ".");
}

/** Build an alias for a column expression, stripping table prefix and quotes. */
function buildColumnAlias(expr: string): string {
  // Get the last part after the last dot (strip table alias prefix)
  const dotParts = expr.split(".");
  const colPart = dotParts[dotParts.length - 1].trim();
  // Strip brackets, backticks, double-quotes
  const clean = colPart.replace(/[[\]"`]/g, "");
  // Handle * — no alias
  if (clean === "*") return "";
  return toPascalCase(clean);
}

/** Prefix column expression with table alias if it doesn't already have one. */
function prefixWithAlias(expr: string, tableAlias: string): string {
  const trimmed = expr.trim();
  // Already has a prefix (contains a dot not inside function parens)
  if (/^[a-zA-Z0-9_]+\./.test(trimmed) || /^\[?\w+\]?\./.test(trimmed)) {
    return trimmed;
  }
  // Skip expressions that are functions, *, numbers, or complex expressions
  if (/^\(/.test(trimmed) || /^\d/.test(trimmed) || trimmed === "*") {
    return trimmed;
  }
  // Skip CASE expressions and other keywords at start
  const firstWord = trimmed.split(/[\s([]/)[0].toUpperCase();
  if (KEYWORDS.has(firstWord)) {
    return trimmed;
  }
  return `${tableAlias}.${trimmed}`;
}


/** Quote an alias value with single quotes (MSSQL style) if not already quoted. */
function quoteAlias(alias: string): string {
  if (/^['"`]/.test(alias)) return alias; // already quoted
  return `'${alias}'`;
}

/** Format a parsed SELECT statement with alignment. */
function formatSelect(parsed: ParsedSelect): string {
  const indent = "    ";
  const lines: string[] = [];

  // Build SELECT line
  let selectLine = "SELECT";
  if (parsed.distinct) selectLine += " DISTINCT";
  if (parsed.top) selectLine += ` TOP ${parsed.top}`;
  lines.push(selectLine);

  // Determine the primary table alias for prefixing
  const primaryAlias = parsed.fromTables.length > 0
    ? (parsed.fromTables[0].alias ?? generateTableAlias(parsed.fromTables[0].expr))
    : null;

  // Ensure all FROM tables have aliases
  const tablesWithAliases = parsed.fromTables.map((t) => ({
    ...t,
    alias: t.alias ?? generateTableAlias(t.expr),
  }));

  // Process columns: add aliases and table prefixes
  const processedCols = parsed.columns.map((col) => {
    let expr = col.expr;

    // Prefix with table alias if single table and no prefix yet
    if (primaryAlias && tablesWithAliases.length === 1) {
      expr = prefixWithAlias(expr, primaryAlias);
    }

    // Generate alias if missing
    let alias = col.alias;
    if (!alias) {
      const generated = buildColumnAlias(expr);
      if (generated) {
        alias = quoteAlias(generated);
      }
    } else {
      // Ensure existing alias has no spaces — convert to PascalCase
      const cleanAlias = alias.replace(/^['"`]|['"`]$/g, "").trim();
      if (cleanAlias.includes(" ")) {
        alias = quoteAlias(toPascalCase(cleanAlias));
      } else {
        alias = quoteAlias(cleanAlias);
      }
    }

    return { expr, alias };
  });

  // Calculate max expression length for alignment
  const maxExprLen = processedCols.reduce(
    (max, col) => Math.max(max, col.expr.length),
    0,
  );

  // Render column lines
  processedCols.forEach((col, i) => {
    const comma = i < processedCols.length - 1 ? "," : "";
    if (col.alias) {
      const padding = " ".repeat(maxExprLen - col.expr.length + 1);
      lines.push(`${indent}${col.expr}${padding}AS ${col.alias}${comma}`);
    } else {
      lines.push(`${indent}${col.expr}${comma}`);
    }
  });

  // FROM clause
  if (tablesWithAliases.length > 0) {
    const fromParts = tablesWithAliases.map((t) => `${t.expr} ${t.alias}`);
    lines.push(`FROM ${fromParts[0]}`);
    for (let i = 1; i < fromParts.length; i++) {
      lines.push(`    ,${fromParts[i]}`);
    }
  }

  // Rest of clauses — preserve body as-is (don't rewrite references)
  for (const clause of parsed.restClauses) {
    const kw = clause.keyword;
    if (clause.body) {
      lines.push(`${kw} ${clause.body}`);
    } else {
      lines.push(kw);
    }
  }

  let result = lines.join("\n");
  if (parsed.trailingSemicolon) result += ";";
  return result;
}

/**
 * Format SQL with column alignment, table aliases, and PascalCase column aliases.
 * Falls back to basic keyword-uppercase formatting for non-SELECT statements.
 */
export function formatSqlAligned(sql: string): string {
  const trimmed = sql.trim();
  if (!trimmed) return sql;

  // Split into statements by semicolons (respecting strings and brackets)
  const statements = splitStatements(trimmed);

  const formatted = statements.map((stmt) => {
    const tokens = tokenize(stmt.trim());
    const parsed = parseSelect(tokens);
    if (parsed) {
      return formatSelect(parsed);
    }
    // Non-SELECT: just uppercase keywords and basic formatting
    return formatBasic(tokens);
  });

  return formatted.join("\n\n");
}

/** Split SQL into statements by semicolons, respecting strings and brackets. */
function splitStatements(sql: string): string[] {
  const stmts: string[] = [];
  let current = "";
  let i = 0;
  let inString = false;
  let stringChar = "";
  let depth = 0;

  while (i < sql.length) {
    const ch = sql[i];

    if (inString) {
      current += ch;
      if (ch === stringChar && sql[i + 1] !== stringChar) {
        inString = false;
      } else if (ch === stringChar && sql[i + 1] === stringChar) {
        current += sql[i + 1];
        i += 2;
        continue;
      }
      i++;
      continue;
    }

    if (ch === "'" || ch === '"') {
      inString = true;
      stringChar = ch;
      current += ch;
      i++;
      continue;
    }

    if (ch === "[") depth++;
    if (ch === "]") depth--;

    if (ch === ";" && depth <= 0) {
      current += ";";
      stmts.push(current.trim());
      current = "";
      i++;
      continue;
    }

    current += ch;
    i++;
  }

  if (current.trim()) stmts.push(current.trim());
  return stmts;
}

/** Basic formatting: uppercase keywords, minimal restructuring. */
function formatBasic(tokens: Token[]): string {
  return tokens
    .map((t) => {
      if (t.type === "keyword") return t.upper;
      if (t.type === "whitespace") return " ";
      return t.value;
    })
    .join("")
    .replace(/ +/g, " ")
    .trim();
}
