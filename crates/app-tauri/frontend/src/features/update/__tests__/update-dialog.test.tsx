// What the dialog lets the user leave: closing during the download cancels it;
// the installer and the ready-to-restart prompt may not be closed — they have
// nothing else to do and hold the only way forward.

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));
vi.mock("@/lib/agent-ipc/sessions", () => ({
  cancelUpdate: vi.fn(async () => {}),
}));

import { UpdateDialog } from "../update-dialog";
import { useUpdateStore, type UpdateState } from "../update-store";
import type { UpdateCheck } from "@/lib/agent-ipc/sessions";

const INFO: UpdateCheck = {
  current: "0.3.0",
  latest: "0.4.0",
  url: "https://example.com/v0.4.0",
  notes: "## 更新",
  is_newer: true,
  asset: { name: "Husk_0.4.0_amd64.deb", size: 2048 },
};

function show(state: UpdateState) {
  useUpdateStore.setState({ state, open: true, snoozed: null, checking: false });
  return render(<UpdateDialog />);
}

describe("update dialog dismissal", () => {
  it("can be closed while the package downloads, and the button says it cancels", () => {
    show({ status: "installing", info: INFO, progress: { phase: "download", downloaded: 1, total: 2 } });
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "取消下载" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "稍后" })).toBeNull();
  });

  it("hides both close affordances while the installer runs", () => {
    show({ status: "installing", info: INFO, progress: { phase: "install", downloaded: 2, total: 2 } });
    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
    expect(screen.queryByRole("button", { name: "稍后" })).toBeNull();
  });

  it("hides them too once the update waits for a restart", () => {
    show({ status: "ready", info: INFO, mode: "installed" });
    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
    expect(screen.getByRole("button", { name: "重启应用" })).toBeInTheDocument();
  });
});
