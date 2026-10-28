import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { renderWithProviders } from "@/test/render";
import { AdvancedFields } from "../advanced-fields";

test("the shared provider/model disclosure opens and closes with the keyboard", async () => {
  const user = userEvent.setup();
  renderWithProviders(<AdvancedFields label="高级设置"><input aria-label="详细字段" /></AdvancedFields>);
  const trigger = screen.getByRole("button", { name: "高级设置" });
  expect(trigger).toHaveAttribute("aria-expanded", "false");
  trigger.focus();
  await user.keyboard("{Enter}");
  expect(trigger).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByRole("textbox", { name: "详细字段" })).toBeInTheDocument();
  await user.keyboard(" ");
  expect(trigger).toHaveAttribute("aria-expanded", "false");
});

test("existing extra fields can start expanded without a separate local style", () => {
  renderWithProviders(<AdvancedFields label="高级设置" defaultOpen><input aria-label="详细字段" /></AdvancedFields>);
  expect(screen.getByRole("button", { name: "高级设置" })).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByRole("textbox", { name: "详细字段" })).toBeInTheDocument();
});
