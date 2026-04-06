import { useState, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X, Maximize2 } from "lucide-react";

export default function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const appWindow = getCurrentWindow();

  useEffect(() => {
    appWindow.isMaximized().then(setMaximized);
    const unlisten = appWindow.onResized(() => {
      appWindow.isMaximized().then(setMaximized);
    });
    return () => { unlisten.then((f) => f()); };
  }, [appWindow]);

  return (
    <div
      className="flex h-8 shrink-0 items-center border-b border-border bg-muted/50 select-none"
      onMouseDown={(e) => {
        // Only drag on the bar itself, not on buttons
        if ((e.target as HTMLElement).closest("button")) return;
        appWindow.startDragging();
      }}
      onDoubleClick={() => appWindow.toggleMaximize()}
    >
      <div className="flex items-center gap-2 px-3 text-xs font-medium text-muted-foreground">
        <img src="/sqail-logo.svg" alt="" className="h-4 w-4" onError={(e) => (e.currentTarget.style.display = "none")} />
        <span>SQaiL v{__APP_VERSION__}</span>
      </div>

      <div className="flex-1" />

      <button
        onClick={() => appWindow.minimize()}
        className="flex h-8 w-10 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="Minimize"
      >
        <Minus size={14} />
      </button>
      <button
        onClick={() => appWindow.toggleMaximize()}
        className="flex h-8 w-10 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title={maximized ? "Restore" : "Maximize"}
      >
        {maximized ? <Square size={12} /> : <Maximize2 size={14} />}
      </button>
      <button
        onClick={() => appWindow.close()}
        className="flex h-8 w-10 items-center justify-center text-muted-foreground transition-colors hover:bg-red-500/80 hover:text-white"
        title="Close"
      >
        <X size={14} />
      </button>
    </div>
  );
}
