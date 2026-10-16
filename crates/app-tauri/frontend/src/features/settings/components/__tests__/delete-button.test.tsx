import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { DeleteButton } from "../delete-button";
import { renderWithProviders } from "@/test/render";

test("a row delete asks in a dialog and runs only on confirm", async () => {
  const user = userEvent.setup();
  const onConfirm = vi.fn();
  renderWithProviders(
    <DeleteButton confirmTitle="删除技能" confirmDescription="确定要删除技能「a」吗？" onConfirm={onConfirm} />,
  );
  await user.click(screen.getByRole("button", { name: "删除" }));
  const dialog = await screen.findByRole("dialog");
  expect(dialog).toHaveTextContent("删除技能");
  expect(dialog).toHaveTextContent("确定要删除技能「a」吗？");
  await user.click(screen.getByRole("button", { name: "取消" }));
  expect(onConfirm).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "删除" }));
  await user.click(await screen.findByRole("button", { name: "删除" }));
  expect(onConfirm).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("the text variant titles the dialog with its label and custom confirm text", async () => {
  const user = userEvent.setup();
  const onConfirm = vi.fn();
  renderWithProviders(
    <DeleteButton variant="text" label="全部清空" confirmLabel="全部清空" onConfirm={onConfirm} />,
  );
  await user.click(screen.getByRole("button", { name: "全部清空" }));
  expect(await screen.findByRole("heading", { name: "全部清空" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "全部清空" }));
  expect(onConfirm).toHaveBeenCalledTimes(1);
});
