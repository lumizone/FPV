import { getCurrentWindow } from "@tauri-apps/api/window";
import { isWindows } from "@/lib/platform";

type Dir =
  | "North" | "South" | "East" | "West"
  | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";

const EDGE = 6;

function Handle({ dir, style }: { dir: Dir; style: React.CSSProperties }) {
  return (
    <div
      aria-hidden="true"
      style={{ position: "fixed", zIndex: 200, ...style }}
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        getCurrentWindow().startResizeDragging(dir);
      }}
    />
  );
}

export function ResizeHandles() {
  if (!isWindows) return null;
  return (
    <>
      <Handle dir="North" style={{ top: 0, left: EDGE, right: EDGE, height: EDGE, cursor: "n-resize" }} />
      <Handle dir="South" style={{ bottom: 0, left: EDGE, right: EDGE, height: EDGE, cursor: "s-resize" }} />
      <Handle dir="West" style={{ left: 0, top: EDGE, bottom: EDGE, width: EDGE, cursor: "w-resize" }} />
      <Handle dir="East" style={{ right: 0, top: EDGE, bottom: EDGE, width: EDGE, cursor: "e-resize" }} />
      <Handle dir="NorthWest" style={{ top: 0, left: 0, width: EDGE, height: EDGE, cursor: "nw-resize" }} />
      <Handle dir="NorthEast" style={{ top: 0, right: 0, width: EDGE, height: EDGE, cursor: "ne-resize" }} />
      <Handle dir="SouthWest" style={{ bottom: 0, left: 0, width: EDGE, height: EDGE, cursor: "sw-resize" }} />
      <Handle dir="SouthEast" style={{ bottom: 0, right: 0, width: EDGE, height: EDGE, cursor: "se-resize" }} />
    </>
  );
}
