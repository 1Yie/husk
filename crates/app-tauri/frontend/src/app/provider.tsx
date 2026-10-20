// 全局 Provider 组合 + 启动副作用。入口只负责挂载，这里负责“画第一帧之前必须做完的事”。
import type { ReactNode } from "react";
import { TooltipProvider, GlobalTooltip } from "@/components/ui/tooltip";
import { getAppearance } from "@/lib/agent-ipc";
import { initAppearance } from "@/lib/appearance";
import { initDesktopNotify } from "@/lib/desktop-notify";
import { initNotificationSound } from "@/lib/notification-sound";
import { installDownloadBridge } from "@/lib/download-bridge";
import { initAgentStore } from "@/stores/agent-store";

// Exports streamdown builds from a blob (diagram SVG/PNG, table CSV, images)
// only reach the filesystem through the native save dialog — install before
// anything can render a download button.
installDownloadBridge();

// The kernel event stream belongs to no component: install its pump now, so the
// first turn after mount cannot land before anyone is listening.
initAgentStore();

// Desktop notifications ride that same stream — read the stored switch now so
// the first turn end already knows whether the shell should be told.
initDesktopNotify();

// …and its audible half: the cue for a finished / blocked / failed loop reads
// its own switch the same way.
initNotificationSound();

// Apply the stored appearance before first paint — the shell reads the `dark`
// class + CSS vars off <html>, so waiting for React to mount would flash the
// default light theme for a frame.
void getAppearance()
  .then((cfg) => initAppearance(cfg))
  .catch(() => {});

export function AppProviders({ children }: { children: ReactNode }) {
  return (
    <TooltipProvider delayDuration={200}>
      {children}
      <GlobalTooltip />
    </TooltipProvider>
  );
}
