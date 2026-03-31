import type { editor as monacoEditor } from "monaco-editor";

interface SqlError {
  line: number;
  column: number;
  endColumn: number;
  message: string;
  severity: "error" | "warning";
}

/**
 * Lightweight SQL validator that catches common syntax issues.
 * Not a full parser — just detects obvious mistakes.
 */
export function validateSql(text: string): SqlError[] {
  const errors: SqlError[] = [];
  const lines = text.split("\n");

  // Track state across lines
  let inSingleQuote = false;
  let inDoubleQuote = false;
  let inLineComment = false;
  let inBlockComment = false;
  let parenDepth = 0;
  const parenStack: { line: number; col: number }[] = [];

  for (let lineIdx = 0; lineIdx < lines.length; lineIdx++) {
    const line = lines[lineIdx];
    inLineComment = false;

    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      const next = line[i + 1];

      // Block comment end
      if (inBlockComment) {
        if (ch === "*" && next === "/") {
          inBlockComment = false;
          i++;
        }
        continue;
      }

      // Line comment
      if (inLineComment) continue;

      // String handling
      if (inSingleQuote) {
        if (ch === "'" && next === "'") { i++; continue; } // escaped quote
        if (ch === "'") inSingleQuote = false;
        continue;
      }
      if (inDoubleQuote) {
        if (ch === '"' && next === '"') { i++; continue; }
        if (ch === '"') inDoubleQuote = false;
        continue;
      }

      // Start contexts
      if (ch === "-" && next === "-") { inLineComment = true; continue; }
      if (ch === "/" && next === "*") { inBlockComment = true; i++; continue; }
      if (ch === "'") { inSingleQuote = true; continue; }
      if (ch === '"') { inDoubleQuote = true; continue; }

      // Paren tracking
      if (ch === "(") {
        parenStack.push({ line: lineIdx + 1, col: i + 1 });
        parenDepth++;
      }
      if (ch === ")") {
        if (parenDepth <= 0) {
          errors.push({
            line: lineIdx + 1,
            column: i + 1,
            endColumn: i + 2,
            message: "Unmatched closing parenthesis",
            severity: "error",
          });
        } else {
          parenStack.pop();
          parenDepth--;
        }
      }
    }

    // Check for unclosed single-line string (not inside block comment)
    if (inSingleQuote && !inBlockComment) {
      // Check if it's actually a multi-line string — SQL doesn't support those
      errors.push({
        line: lineIdx + 1,
        column: 1,
        endColumn: line.length + 1,
        message: "Unclosed string literal (single quote)",
        severity: "error",
      });
      inSingleQuote = false; // reset to avoid cascading errors
    }
  }

  // Unclosed block comment
  if (inBlockComment) {
    errors.push({
      line: lines.length,
      column: 1,
      endColumn: lines[lines.length - 1].length + 1,
      message: "Unclosed block comment (missing */)",
      severity: "error",
    });
  }

  // Unmatched opening parens
  for (const p of parenStack) {
    errors.push({
      line: p.line,
      column: p.col,
      endColumn: p.col + 1,
      message: "Unmatched opening parenthesis",
      severity: "error",
    });
  }

  // Statement-level checks on non-comment, non-empty content
  checkStatements(text, errors);

  return errors;
}

/** Basic statement-level checks */
function checkStatements(text: string, errors: SqlError[]) {
  // Strip comments and strings for keyword analysis
  const stripped = stripCommentsAndStrings(text);
  const lines = text.split("\n");

  // Find SELECT without FROM (unless it's just SELECT expression like SELECT 1)
  // Find INSERT without INTO, UPDATE without SET, DELETE without FROM
  const stmts = splitStatements(stripped);

  for (const stmt of stmts) {
    const upper = stmt.text.toUpperCase().trim();
    if (!upper) continue;

    if (upper.startsWith("SELECT") && !upper.includes("FROM")) {
      // Check if it's a simple expression select (SELECT 1, SELECT GETDATE(), etc.)
      const afterSelect = upper.slice(6).trim();
      if (afterSelect.includes(",") || afterSelect.length > 50) {
        // Likely expects a FROM — warn
        const lineIdx = findKeywordLine(lines, stmt.startLine, "SELECT");
        if (lineIdx >= 0) {
          errors.push({
            line: lineIdx + 1,
            column: 1,
            endColumn: lines[lineIdx].length + 1,
            message: "SELECT without FROM clause",
            severity: "warning",
          });
        }
      }
    }

    if (upper.startsWith("INSERT") && !upper.includes("INTO")) {
      errors.push({
        line: stmt.startLine + 1,
        column: 1,
        endColumn: lines[stmt.startLine]?.length + 1 || 1,
        message: "INSERT without INTO",
        severity: "error",
      });
    }

    if (upper.startsWith("UPDATE") && !upper.includes("SET")) {
      errors.push({
        line: stmt.startLine + 1,
        column: 1,
        endColumn: lines[stmt.startLine]?.length + 1 || 1,
        message: "UPDATE without SET clause",
        severity: "error",
      });
    }
  }
}

function stripCommentsAndStrings(text: string): string {
  let result = "";
  let inSQ = false, inDQ = false, inLC = false, inBC = false;

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    const next = text[i + 1];

    if (inBC) {
      if (ch === "*" && next === "/") { inBC = false; result += "  "; i++; } else { result += " "; }
      continue;
    }
    if (inLC) {
      if (ch === "\n") { inLC = false; result += ch; } else { result += " "; }
      continue;
    }
    if (inSQ) {
      if (ch === "'" && next === "'") { result += "  "; i++; continue; }
      if (ch === "'") { inSQ = false; result += ch; } else { result += " "; }
      continue;
    }
    if (inDQ) {
      if (ch === '"' && next === '"') { result += "  "; i++; continue; }
      if (ch === '"') { inDQ = false; result += ch; } else { result += " "; }
      continue;
    }

    if (ch === "-" && next === "-") { inLC = true; result += " "; continue; }
    if (ch === "/" && next === "*") { inBC = true; result += " "; i++; continue; }
    if (ch === "'") { inSQ = true; result += ch; continue; }
    if (ch === '"') { inDQ = true; result += ch; continue; }
    result += ch;
  }
  return result;
}

function splitStatements(stripped: string): { text: string; startLine: number }[] {
  const stmts: { text: string; startLine: number }[] = [];
  let current = "";
  let startLine = 0;
  let lineNum = 0;

  for (const ch of stripped) {
    if (ch === "\n") lineNum++;
    if (ch === ";") {
      if (current.trim()) stmts.push({ text: current, startLine });
      current = "";
      startLine = lineNum + 1;
    } else {
      if (!current.trim() && ch.trim()) startLine = lineNum;
      current += ch;
    }
  }
  if (current.trim()) stmts.push({ text: current, startLine });
  return stmts;
}

function findKeywordLine(lines: string[], startLine: number, keyword: string): number {
  for (let i = startLine; i < lines.length; i++) {
    if (lines[i].toUpperCase().includes(keyword)) return i;
  }
  return startLine;
}

/** Convert SqlErrors to Monaco markers */
export function toMonacoMarkers(errors: SqlError[]): monacoEditor.IMarkerData[] {
  return errors.map((e) => ({
    startLineNumber: e.line,
    startColumn: e.column,
    endLineNumber: e.line,
    endColumn: e.endColumn,
    message: e.message,
    severity: e.severity === "error" ? 8 : 4, // MarkerSeverity.Error : MarkerSeverity.Warning
  }));
}
