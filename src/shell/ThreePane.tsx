/**
 * Three-pane layout (S3): tree panel | list panel | detail pane, resizable
 * by dragging the splitters, widths persisted per module in localStorage.
 */
import { useEffect, useRef, useState } from "preact/hooks";
import { type ModuleId } from "../store";

function loadWidths(m: ModuleId): { tree: number; list: number } {
  try {
    const raw = localStorage.getItem(`pf.pane.${m}`);
    if (raw) return JSON.parse(raw) as { tree: number; list: number };
  } catch {
    /* fall through to defaults */
  }
  return { tree: 220, list: 380 };
}

export function ThreePane({
  module,
  tree,
  list,
  detail,
}: {
  module: ModuleId;
  tree: preact.ComponentChildren;
  list: preact.ComponentChildren;
  detail: preact.ComponentChildren;
}) {
  const [{ tree: treeW, list: listW }, setW] = useState(() => loadWidths(module));
  const root = useRef<HTMLDivElement>(null);
  const dragging = useRef<null | "tree" | "list">(null);

  useEffect(() => setW(loadWidths(module)), [module]);
  useEffect(() => {
    localStorage.setItem(`pf.pane.${module}`, JSON.stringify({ tree: treeW, list: listW }));
  }, [module, treeW, listW]);

  useEffect(() => {
    const move = (e: MouseEvent) => {
      if (!dragging.current || !root.current) return;
      const rect = root.current.getBoundingClientRect();
      const x = e.clientX - rect.left;
      if (dragging.current === "tree") {
        setW((w) => ({ ...w, tree: Math.min(Math.max(x, 160), 420) }));
      } else {
        setW((w) => ({
          ...w,
          list: Math.min(Math.max(x - w.tree - 8, 240), rect.width - 380),
        }));
      }
    };
    const up = () => (dragging.current = null);
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
    return () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
    };
  }, []);

  return (
    <div class="threepane" ref={root}>
      <aside class="pane pane-tree" style={{ width: `${treeW}px` }}>{tree}</aside>
      <div
        class="splitter"
        role="separator"
        aria-orientation="vertical"
        onMouseDown={() => (dragging.current = "tree")}
      />
      <section class="pane pane-list" style={{ width: `${listW}px` }}>{list}</section>
      <div
        class="splitter"
        role="separator"
        aria-orientation="vertical"
        onMouseDown={() => (dragging.current = "list")}
      />
      <section class="pane pane-detail">{detail}</section>
    </div>
  );
}
