import { useState, useRef, useCallback } from "react";

interface ResizablePanelProps {
  top: React.ReactNode;
  bottom: React.ReactNode;
  defaultRatio?: number; // 0-1, portion for top panel
  minTopHeight?: number;
  minBottomHeight?: number;
}

export default function ResizablePanel({
  top,
  bottom,
  defaultRatio = 0.6,
  minTopHeight = 100,
  minBottomHeight = 80,
}: ResizablePanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [ratio, setRatio] = useState(defaultRatio);
  const dragging = useRef(false);

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragging.current = true;

    const onMouseMove = (e: MouseEvent) => {
      if (!dragging.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const totalHeight = rect.height;
      const y = e.clientY - rect.top;
      const clamped = Math.max(
        minTopHeight / totalHeight,
        Math.min(y / totalHeight, 1 - minBottomHeight / totalHeight),
      );
      setRatio(clamped);
    };

    const onMouseUp = () => {
      dragging.current = false;
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  }, [minTopHeight, minBottomHeight]);

  return (
    <div ref={containerRef} className="flex flex-1 flex-col overflow-hidden">
      <div style={{ flex: `${ratio} 1 0%` }} className="flex min-h-0 flex-col overflow-hidden">
        {top}
      </div>
      <div
        onMouseDown={onMouseDown}
        className="flex h-1.5 shrink-0 cursor-row-resize items-center justify-center bg-border/50 hover:bg-primary/30 transition-colors"
      >
        <div className="h-0.5 w-8 rounded-full bg-muted-foreground/30" />
      </div>
      <div style={{ flex: `${1 - ratio} 1 0%` }} className="flex min-h-0 flex-col overflow-hidden">
        {bottom}
      </div>
    </div>
  );
}
