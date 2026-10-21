/**
 * Virtualized list (T2.8 / AT-19): fixed-height rows, windowed mounting via
 * visibleRange(), absolute positioning with a transform — no layout thrash,
 * no library adapter. Only the mounted window exists in the DOM even with
 * 100k rows behind it.
 */
import type { JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { visibleRange } from "../lib/list-model";

export interface VirtualListProps {
  /** Total number of rows (mounted or not). */
  rowCount: number;
  rowHeight: number;
  overscan?: number;
  /** Render one row by index; called only for mounted rows. */
  renderRow: (index: number) => JSX.Element;
  className?: string;
  /** Fired when the user scrolls near the bottom — hook for the next page. */
  onNearEnd?: () => void;
}

export function VirtualList({
  rowCount,
  rowHeight,
  overscan = 6,
  renderRow,
  className = "",
  onNearEnd,
}: VirtualListProps) {
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(600);
  const el = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const node = el.current;
    if (!node) return;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) setViewport(e.contentRect.height);
    });
    ro.observe(node);
    setViewport(node.clientHeight);
    return () => ro.disconnect();
  }, []);

  const onScroll = () => {
    const node = el.current;
    if (!node) return;
    setScrollTop(node.scrollTop);
    if (onNearEnd && node.scrollTop + node.clientHeight >= node.scrollHeight - rowHeight * 10) {
      onNearEnd();
    }
  };

  const { start, end } = visibleRange(scrollTop, viewport, rowHeight, rowCount, overscan);
  const rows: JSX.Element[] = [];
  for (let i = start; i < end; i++) {
    rows.push(renderRow(i));
  }

  return (
    <div class={`vlist ${className}`} ref={el} onScroll={onScroll}>
      <div class="vlist-inner" style={{ height: `${rowCount * rowHeight}px` }}>
        <div style={{ transform: `translateY(${start * rowHeight}px)` }}>
          {rows}
        </div>
      </div>
    </div>
  );
}
