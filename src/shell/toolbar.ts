/**
 * App-level toolbar slot (EfficientPIM-style: one toolbar under the menu
 * bar). The active module registers its toolbar content here on mount and
 * clears it on unmount — the shell renders it, the module keeps its handlers.
 */
import { signal } from "@preact/signals";
import { useEffect } from "preact/hooks";
import type { ComponentChildren } from "preact";

export const toolbarSlot = signal<ComponentChildren>(null);

/**
 * Register this module's toolbar content for as long as the module is
 * mounted. Re-registers whenever any dep changes so closures stay fresh.
 */
export function useToolbar(content: ComponentChildren, deps: unknown[]): void {
  useEffect(() => {
    toolbarSlot.value = content;
    return () => {
      toolbarSlot.value = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
