import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";
import {
  ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSub,
  ContextMenuSubContent, ContextMenuSubTrigger, ContextMenuTrigger,
} from "../context-menu";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSub,
  DropdownMenuSubContent, DropdownMenuSubTrigger, DropdownMenuTrigger,
} from "../dropdown-menu";

describe("portalled submenus", () => {
  test.each(["context", "dropdown"] as const)("%s submenu escapes the parent and workspace clip", async (kind) => {
    const user = userEvent.setup();
    const select = vi.fn();
    render(
      <section data-testid="workspace-clip" style={{ overflow: "hidden" }}>
        {kind === "context" ? (
          <ContextMenu>
            <ContextMenuTrigger>会话</ContextMenuTrigger>
            <ContextMenuContent data-testid="parent-menu">
              <ContextMenuSub defaultOpen>
                <ContextMenuSubTrigger>置顶</ContextMenuSubTrigger>
                <ContextMenuSubContent data-testid="submenu">
                  <ContextMenuItem onSelect={select}>在工作区置顶</ContextMenuItem>
                </ContextMenuSubContent>
              </ContextMenuSub>
            </ContextMenuContent>
          </ContextMenu>
        ) : (
          <DropdownMenu>
            <DropdownMenuTrigger>会话</DropdownMenuTrigger>
            <DropdownMenuContent data-testid="parent-menu">
              <DropdownMenuSub defaultOpen>
                <DropdownMenuSubTrigger>置顶</DropdownMenuSubTrigger>
                <DropdownMenuSubContent data-testid="submenu">
                  <DropdownMenuItem onSelect={select}>在工作区置顶</DropdownMenuItem>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </section>,
    );
    if (kind === "context") fireEvent.contextMenu(screen.getByText("会话"), { clientX: 40, clientY: 40 });
    else await user.click(screen.getByRole("button", { name: "会话" }));
    await user.click(await screen.findByRole("menuitem", { name: "置顶" }));
    const child = await screen.findByTestId("submenu");
    expect(screen.getByTestId("parent-menu").contains(child)).toBe(false);
    expect(screen.getByTestId("workspace-clip").contains(child)).toBe(false);
    screen.getByRole("menuitem", { name: "在工作区置顶" }).focus();
    await user.keyboard("{ArrowLeft}");
    await waitFor(() => expect(screen.queryByTestId("submenu")).toBeNull());
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByTestId("parent-menu")).toBeNull());
    if (kind === "context") fireEvent.contextMenu(screen.getByText("会话"), { clientX: 40, clientY: 40 });
    else await user.click(screen.getByRole("button", { name: "会话" }));
    await user.click(await screen.findByRole("menuitem", { name: "置顶" }));
    await screen.findByTestId("submenu");
    screen.getByRole("menuitem", { name: "在工作区置顶" }).focus();
    await user.keyboard("{Enter}");
    expect(select).toHaveBeenCalledTimes(1);
  });
});
