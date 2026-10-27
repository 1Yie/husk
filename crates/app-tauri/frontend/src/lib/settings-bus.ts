/**
 * Settings bus — 设置 is an overlay the shell owns (App.tsx), so anything
 * inside the app can ask for it, on a specific page, without threading state
 * through the whole tree.
 */

/** The pages a local command can land on. */
export type SettingsTarget = "general" | "skills" | "memory" | "terminal";

const EVENT = "husk:open-settings";

/** Ask the shell to open 设置 on `page`. */
export function openSettings(page: SettingsTarget = "general") {
 window.dispatchEvent(
  new CustomEvent<{ page: SettingsTarget }>(EVENT, { detail: { page } }),
 );
}

/** Shell side: subscribe, get the requested page. */
export function onOpenSettings(
 handler: (page: SettingsTarget) => void,
): () => void {
 const listener = (event: Event) => {
  const page = (event as CustomEvent<{ page?: SettingsTarget }>).detail?.page;
  handler(page ?? "general");
 };
 window.addEventListener(EVENT, listener);
 return () => window.removeEventListener(EVENT, listener);
}
