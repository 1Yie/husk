import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { GeneralPane } from "../index";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";

test("changing 审批超时 stores the bound in seconds", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ desktop_notifications: true, approval_timeout_secs: 300, question_timeout_secs: 600 }),
    "get_default_prefs",
  );
  mockCommand("agent_session", () => ({ config: { providers: {} } }), "get_app_config");
  mockCommand("agent_session", () => ({ success: true }), "set_default_prefs");
  renderWithProviders(<GeneralPane />);

  // The stored bound is what the row shows before anything is touched.
  const select = await screen.findByRole("button", { name: "审批超时" });
  expect(select).toHaveTextContent("5 分钟");

  await user.click(select);
  await user.click(await screen.findByRole("menuitemradio", { name: "2 分钟" }));
  // Minutes are the label; the host takes seconds.
  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "set_default_prefs",
      payload: { approval_timeout_secs: 120 },
    }),
  );
  expect(select).toHaveTextContent("2 分钟");
});
