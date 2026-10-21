/**
 * Shell toolbar: renders the active module's registered toolbar content
 * (see toolbar.ts) in a full-width bar under the menu bar. Renders nothing
 * while no module has registered content (Today).
 */
import { toolbarSlot } from "./toolbar";

export function AppToolbar() {
  if (toolbarSlot.value === null) return null;
  return (
    <div class="app-toolbar" role="toolbar" aria-label="Module actions">
      {toolbarSlot.value}
    </div>
  );
}
