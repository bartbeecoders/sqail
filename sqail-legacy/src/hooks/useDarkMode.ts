import { useSyncExternalStore } from "react";
import { useSettingsStore } from "../stores/settingsStore";

function subscribe(callback: () => void) {
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  mql.addEventListener("change", callback);
  return () => mql.removeEventListener("change", callback);
}

function getSnapshot() {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function useDarkMode(): boolean {
  const systemDark = useSyncExternalStore(subscribe, getSnapshot);
  const theme = useSettingsStore((s) => s.theme);

  if (theme === "dark") return true;
  if (theme === "light") return false;
  return systemDark;
}
