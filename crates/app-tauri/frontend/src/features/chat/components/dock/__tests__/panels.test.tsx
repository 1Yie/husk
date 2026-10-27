import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { renderWithProviders } from "@/test/render";
import { ArtifactsList } from "../../artifacts-panel";
import { JobsList } from "../../jobs-panel";
import type { BgJob } from "@/features/chat/hooks/stream-view";

const ipc = vi.hoisted(() => ({
  open: vi.fn(),
  save: vi.fn(),
  reveal: vi.fn(),
  logs: vi.fn(),
  control: vi.fn(),
}));

vi.mock("@/lib/agent-ipc/index", async (original) => ({
  ...await original<object>(),
  openPath: ipc.open,
  saveArtifact: ipc.save,
  revealPath: ipc.reveal,
}));

vi.mock("@/lib/agent-ipc/sessions", async (original) => ({
  ...await original<object>(),
  bgJobLogs: ipc.logs,
  controlBgJob: ipc.control,
}));

const job: BgJob = {
  id: "bg-compact",
  command: "echo ready",
  state: { kind: "running" },
  started_at_ms: 1,
  elapsed_ms: 1000,
  notify: true,
  has_own_sink: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  ipc.open.mockResolvedValue(undefined);
  ipc.save.mockResolvedValue(undefined);
  ipc.reveal.mockResolvedValue(undefined);
  ipc.logs.mockResolvedValue("");
  ipc.control.mockResolvedValue({ jobs: [] });
});

test("artifact row retains keyboard open and all three file actions", async () => {
  renderWithProviders(
    <ArtifactsList artifacts={[{
      path: "/work/report.md", name: "report.md", ext: "md",
      tool: "write", updatedAt: 0,
    }]} />,
  );
  const row = screen.getByText("report.md").closest("[role=button]") as HTMLElement;
  fireEvent.keyDown(row, { key: "Enter" });
  await waitFor(() => expect(ipc.open).toHaveBeenCalledWith("/work/report.md"));
  const actions = within(row).getAllByRole("button");
  expect(actions).toHaveLength(3);
  await act(async () => {
    for (const action of actions) fireEvent.click(action);
  });
  expect(ipc.open).toHaveBeenCalledTimes(2);
  expect(ipc.save).toHaveBeenCalledWith("/work/report.md");
  expect(ipc.reveal).toHaveBeenCalledWith("/work/report.md");
});

test("expanded running job settles to no output and stop does not toggle the row", async () => {
  let finish!: (tail: string) => void;
  ipc.logs.mockReturnValueOnce(new Promise<string>((resolve) => { finish = resolve; }));
  const onJobs = vi.fn();
  renderWithProviders(
    <JobsList jobs={[job]} root="/work" session={7} snapshotAt={1} onJobs={onJobs} />,
  );
  const row = screen.getByRole("button", { name: /bg-compact/ });
  fireEvent.keyDown(row, { key: "Enter" });
  expect(row).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByRole("status", { name: "正在读取后台日志" })).toBeInTheDocument();
  await act(async () => finish(""));
  expect(screen.getByText("（暂无输出）")).toBeInTheDocument();
  expect(screen.queryByRole("status", { name: "正在读取后台日志" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "终止" }));
  await waitFor(() => expect(onJobs).toHaveBeenCalledWith([]));
  expect(ipc.control).toHaveBeenCalledWith("/work", 7, "bg-compact", "kill");
  expect(row).toHaveAttribute("aria-expanded", "true");
});

test("finished jobs retain individual removal and bulk cleanup", async () => {
  const done: BgJob = {
    ...job, state: { kind: "done", label: "exited(0)", code: 0, reason: "exited" },
  };
  const onJobs = vi.fn();
  renderWithProviders(
    <JobsList jobs={[done]} root="/work" session={7} snapshotAt={1} onJobs={onJobs} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "移除" }));
  await waitFor(() => expect(onJobs).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "清除已结束" }));
  await waitFor(() => expect(onJobs).toHaveBeenCalledTimes(2));
  expect(ipc.control).toHaveBeenNthCalledWith(1, "/work", 7, "bg-compact", "remove");
  expect(ipc.control).toHaveBeenNthCalledWith(2, "/work", 7, "bg-compact", "remove");
});
