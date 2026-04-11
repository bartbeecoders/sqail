import { useEffect, useState, type CSSProperties } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

// Tauri's ResizeDirection enum values (string form)
type ResizeDir =
  | "North"
  | "South"
  | "East"
  | "West"
  | "NorthEast"
  | "NorthWest"
  | "SouthEast"
  | "SouthWest";

const EDGE = 4; // straight-edge thickness in px
const CORNER = 8; // corner handle size in px

const HANDLES: { dir: ResizeDir; style: CSSProperties; cursor: string }[] = [
  { dir: "North",     style: { top: 0,        left: CORNER,  right: CORNER,  height: EDGE   }, cursor: "ns-resize"   },
  { dir: "South",     style: { bottom: 0,     left: CORNER,  right: CORNER,  height: EDGE   }, cursor: "ns-resize"   },
  { dir: "West",      style: { left: 0,       top: CORNER,   bottom: CORNER, width: EDGE    }, cursor: "ew-resize"   },
  { dir: "East",      style: { right: 0,      top: CORNER,   bottom: CORNER, width: EDGE    }, cursor: "ew-resize"   },
  { dir: "NorthWest", style: { top: 0,        left: 0,       width: CORNER,  height: CORNER }, cursor: "nwse-resize" },
  { dir: "NorthEast", style: { top: 0,        right: 0,      width: CORNER,  height: CORNER }, cursor: "nesw-resize" },
  { dir: "SouthWest", style: { bottom: 0,     left: 0,       width: CORNER,  height: CORNER }, cursor: "nesw-resize" },
  { dir: "SouthEast", style: { bottom: 0,     right: 0,      width: CORNER,  height: CORNER }, cursor: "nwse-resize" },
];

export default function ResizeHandles() {
  const [maximized, setMaximized] = useState(false);
  const appWindow = getCurrentWindow();

  useEffect(() => {
    appWindow.isMaximized().then(setMaximized);
    const unlisten = appWindow.onResized(() => {
      appWindow.isMaximized().then(setMaximized);
    });
    return () => { unlisten.then((f) => f()); };
  }, [appWindow]);

  // No resize handles while maximized — the OS treats the window as fixed.
  if (maximized) return null;

  return (
    <>
      {HANDLES.map(({ dir, style, cursor }) => (
        <div
          key={dir}
          className="fixed z-[9999]"
          style={{ ...style, cursor }}
          onMouseDown={(e) => {
            e.preventDefault();
            e.stopPropagation();
            // Cast: Tauri's ResizeDirection type isn't re-exported publicly,
            // but the runtime accepts the matching string values.
            (appWindow.startResizeDragging as (d: string) => Promise<void>)(dir);
          }}
        />
      ))}
    </>
  );
}
