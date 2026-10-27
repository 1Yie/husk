import { act, render, screen, waitForElementToBeRemoved } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { toast } from "sonner";
import { Toaster } from "../sonner";

const longError = "provider failure: " + "stack-frame-with-an-unbroken-identifier".repeat(200);

afterEach(() => {
  act(() => { toast.dismiss(); });
});

test("long titles and descriptions share a bounded, scrollable content region", async () => {
  const title = "子代理出错。".repeat(100);
  render(<Toaster />);
  act(() => {
    toast.error(title, { description: longError, duration: Infinity });
  });

  const heading = await screen.findByText(title);
  const content = heading.closest("[data-content]");
  expect(content).toHaveClass(
    "min-w-0",
    "max-h-[min(12rem,calc(100dvh-6rem))]",
    "overflow-y-auto",
    "overscroll-contain",
    "[overflow-wrap:anywhere]",
  );
  expect(heading.textContent).toBe(title);
  expect(content?.querySelector("[data-description]")?.textContent).toBe(longError);
});

test("actions stay outside the scrolling error region and remain usable", async () => {
  const onAction = vi.fn();
  const user = userEvent.setup();
  render(<Toaster />);
  act(() => {
    toast.error("后台任务失败", {
      description: longError,
      duration: Infinity,
      action: { label: "查看任务", onClick: onAction },
    });
  });

  const heading = await screen.findByText("后台任务失败");
  const button = screen.getByRole("button", { name: "查看任务" });
  expect(heading.closest("[data-content]")).not.toContainElement(button);
  await user.click(button);
  expect(onAction).toHaveBeenCalledOnce();
});

test("a close button is not clipped by the scrolling content and dismisses the toast", async () => {
  const user = userEvent.setup();
  render(<Toaster closeButton />);
  act(() => {
    toast.error("可关闭的错误", { description: longError, duration: Infinity });
  });

  const heading = await screen.findByText("可关闭的错误");
  const close = heading.closest("[data-sonner-toast]")
    ?.querySelector<HTMLButtonElement>("button[data-close-button]") ?? null;
  expect(close).toBeInTheDocument();
  expect(heading.closest("[data-content]")).not.toContainElement(close);
  await user.click(close as HTMLButtonElement);
  await waitForElementToBeRemoved(() => screen.queryByText("可关闭的错误"));
});
