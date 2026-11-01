import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { useLayoutStore } from "@/app/layout/layout-store";
import { bindingLabel } from "@/lib/shortcuts";
import { defaultBindings } from "@/lib/shortcuts";
import { WorkspaceTools } from "../workspace-tools";

beforeEach(() => {
  useLayoutStore.setState({ terminalOpen: false });
});

test("workspace actions retain their behavior and the full root remains available", () => {
  const onFiles = vi.fn();
  const onReview = vi.fn();
  const root = "/home/user/projects/a-very-long-workspace-name";
  render(<WorkspaceTools root={root} hasChanges onFiles={onFiles} onReview={onReview} />);

  expect(screen.getByRole("heading", { name: "工作区工具" })).toBeVisible();
  expect(screen.getByTitle(root)).toHaveTextContent(root);
  fireEvent.click(screen.getByRole("button", { name: "预览工作区文件" }));
  fireEvent.click(screen.getByRole("button", { name: "查看工作区改动" }));
  expect(onFiles).toHaveBeenCalledOnce();
  expect(onReview).toHaveBeenCalledOnce();

  const terminal = screen.getByRole("button", { name: "打开终端" });
  expect(terminal.querySelector("kbd")).toHaveTextContent(bindingLabel(defaultBindings().terminal));
  fireEvent.click(terminal);
  expect(useLayoutStore.getState().terminalOpen).toBe(true);
});

test("empty workspace actions stay disabled except for opening a terminal", () => {
  render(<WorkspaceTools root="" hasChanges={false} onFiles={vi.fn()} onReview={vi.fn()} />);
  expect(screen.getByRole("button", { name: "预览工作区文件" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "查看工作区改动" })).toBeDisabled();
  expect(screen.getByText("当前没有未提交改动")).toBeVisible();
  expect(screen.getByRole("button", { name: "打开终端" })).toBeEnabled();
});

test("the file entry can select a workspace before previewing", () => {
  const onOpenWorkspace = vi.fn();
  const onFiles = vi.fn();
  render(<WorkspaceTools root="" hasChanges={false} onFiles={onFiles} onReview={vi.fn()} onOpenWorkspace={onOpenWorkspace} />);
  fireEvent.click(screen.getByRole("button", { name: "预览工作区文件" }));
  expect(onOpenWorkspace).toHaveBeenCalledOnce();
  expect(onFiles).not.toHaveBeenCalled();
});

test("office mode offers read-only files and terminal while keeping changes unavailable", () => {
  const onFiles = vi.fn();
  const onReview = vi.fn();
  render(<WorkspaceTools root="/workspace" hasChanges onFiles={onFiles} onReview={onReview} officeMode />);
  expect(screen.getAllByRole("button")).toHaveLength(2);
  fireEvent.click(screen.getByRole("button", { name: "预览工作区文件" }));
  expect(onFiles).toHaveBeenCalledOnce();
  const terminal = screen.getByRole("button", { name: "打开终端" });
  expect(terminal).toBeVisible();
  fireEvent.click(terminal);
  expect(useLayoutStore.getState().terminalOpen).toBe(true);
  expect(screen.queryByRole("button", { name: "查看工作区改动" })).toBeNull();
  expect(onReview).not.toHaveBeenCalled();
});
