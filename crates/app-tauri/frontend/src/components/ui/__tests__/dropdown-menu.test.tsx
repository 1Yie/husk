import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode, useState } from "react";
import { describe, expect, test } from "vitest";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../dropdown-menu";
import { Dialog, DialogContent, DialogTitle } from "../dialog";

function Menu({ name }: { name: string }) {
  const [open, setOpen] = useState(false);
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger id={`custom-${name}`}>{name}</DropdownMenuTrigger>
      <DropdownMenuContent aria-label={`${name} options`}>
        <DropdownMenuItem>{name} option</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

describe("dropdown trigger focus", () => {
  test("late focus on its own trigger does not dismiss the menu", async () => {
    const user = userEvent.setup();
    render(<StrictMode><Menu name="first" /></StrictMode>);
    const trigger = screen.getByRole("button", { name: "first" });
    await user.click(trigger);
    await screen.findByRole("menu");
    // Some native focus arrives after pointerdown has already opened the menu.
    act(() => trigger.focus());
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("menu")).toHaveAttribute("data-state", "open");
  });

  test("late trigger focus inside a dialog stays open but focus elsewhere dismisses it", async () => {
    const user = userEvent.setup();
    render(
      <Dialog defaultOpen>
        <DialogContent aria-describedby={undefined}>
          <DialogTitle>Provider</DialogTitle>
          <input aria-label="provider name" />
          <Menu name="compatibility" />
        </DialogContent>
      </Dialog>,
    );
    const trigger = screen.getByRole("button", { name: "compatibility" });
    await user.click(trigger);
    await screen.findByRole("menu");
    act(() => trigger.focus());
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    act(() => screen.getByRole("textbox", { name: "provider name" }).focus());
    await waitFor(() => expect(trigger).toHaveAttribute("aria-expanded", "false"));
  });

  test("one press switches menus and the trigger can close and immediately reopen", async () => {
    const user = userEvent.setup();
    render(<><Menu name="first" /><Menu name="second" /></>);
    const first = screen.getByRole("button", { name: "first" });
    const second = screen.getByRole("button", { name: "second" });
    await user.click(first);
    await screen.findByRole("menu", { name: "first options" });
    await user.click(second);
    expect(first).toHaveAttribute("aria-expanded", "false");
    expect(second).toHaveAttribute("aria-expanded", "true");
    await user.click(second);
    expect(second).toHaveAttribute("aria-expanded", "false");
    await user.click(second);
    expect(second).toHaveAttribute("aria-expanded", "true");
    fireEvent.pointerDown(document.body, { button: 0 });
    await waitFor(() => expect(second).toHaveAttribute("aria-expanded", "false"));
  });

  test("keyboard activation can select an item, reopen and dismiss with Escape", async () => {
    const user = userEvent.setup();
    render(<Menu name="first" />);
    const trigger = screen.getByRole("button", { name: "first" });
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    await waitFor(() => expect(screen.getByRole("menuitem")).toHaveFocus());
    await user.keyboard("{Enter}");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    act(() => trigger.focus());
    await user.keyboard("{Enter}");
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    await user.keyboard("{Escape}");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });
});
