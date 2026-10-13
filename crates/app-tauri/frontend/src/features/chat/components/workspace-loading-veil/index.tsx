// WorkspaceLoadingVeil — the boot splash, shown again for workspace/session
// switches. Same markup and the same `#husk-splash` rules as index.html, so
// the two can't drift: the look is defined once, there.
//
// Covers the whole layout (sidebar + conversation) so the two appear together.
// It stays inside the app's stacking order: above the layout and the settings
// overlay, below Radix popups and dialogs.

import iconUrl from "@/assets/husk-icon.png";

export function WorkspaceLoadingVeil({ visible }: { visible: boolean }) {
  return (
    <div
      id="husk-splash"
      className={visible ? "husk-veil-visible" : "leaving"}
      role="status"
      aria-hidden={!visible}
      aria-label={visible ? "加载中" : undefined}
    >
      <img className="icon ready" src={iconUrl} alt="" />
      <div className="bar" />
    </div>
  );
}
