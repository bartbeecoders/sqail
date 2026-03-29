import type { editor } from "monaco-editor";

export const sqlaiDark: editor.IStandaloneThemeData = {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "keyword", foreground: "7c9cf5" },
    { token: "string", foreground: "a5d6a7" },
    { token: "number", foreground: "f5c27c" },
    { token: "comment", foreground: "6a737d", fontStyle: "italic" },
    { token: "operator", foreground: "c9d1d9" },
  ],
  colors: {
    "editor.background": "#1a1a1a",
    "editor.foreground": "#e0e0e0",
    "editor.lineHighlightBackground": "#252525",
    "editor.selectionBackground": "#3a3d5c",
    "editorCursor.foreground": "#7c9cf5",
    "editorLineNumber.foreground": "#555555",
    "editorLineNumber.activeForeground": "#999999",
  },
};

export const sqlaiLight: editor.IStandaloneThemeData = {
  base: "vs",
  inherit: true,
  rules: [
    { token: "keyword", foreground: "4338ca" },
    { token: "string", foreground: "16803c" },
    { token: "number", foreground: "b45309" },
    { token: "comment", foreground: "9ca3af", fontStyle: "italic" },
    { token: "operator", foreground: "374151" },
  ],
  colors: {
    "editor.background": "#ffffff",
    "editor.foreground": "#1f2937",
    "editor.lineHighlightBackground": "#f9fafb",
    "editor.selectionBackground": "#dbeafe",
    "editorCursor.foreground": "#4338ca",
    "editorLineNumber.foreground": "#d1d5db",
    "editorLineNumber.activeForeground": "#6b7280",
  },
};
