import { fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test } from "vitest";
import { renderWithProviders } from "@/test/render";
import { isMac } from "@/lib/platform";
import { bindingLabel, defaultBindings } from "@/lib/shortcuts";
import { useShortcutStore } from "@/lib/shortcuts-store";
import { ShortcutsSettings } from "..";

const modifier = isMac ? { metaKey: true } : { ctrlKey: true };

beforeEach(() => {
  localStorage.clear();
  useShortcutStore.setState({ bindings: defaultBindings() });
});

test("a row captures the next combination and persists it", async () => {
  const user = userEvent.setup();
  renderWithProviders(<ShortcutsSettings />);
  const row = screen.getByRole("button", { name: "重新绑定打开／收起终端" });
  expect(screen.getByLabelText("打开／收起终端的当前快捷键")).toHaveTextContent(bindingLabel("mod+shift+j"));

  await user.click(row);
  expect(screen.getByLabelText("打开／收起终端的当前快捷键")).toHaveTextContent("按下组合…");

  fireEvent.keyDown(window, { key: "k", shiftKey: true, ...modifier });
  expect(useShortcutStore.getState().bindings.terminal).toBe("mod+shift+k");
  expect(screen.getByLabelText("打开／收起终端的当前快捷键")).toHaveTextContent(bindingLabel("mod+shift+k"));
});

test("a combination already in use is refused and the row keeps its binding", async () => {
  const user = userEvent.setup();
  renderWithProviders(<ShortcutsSettings />);
  await user.click(screen.getByRole("button", { name: "重新绑定打开／收起终端" }));
  // mod+shift+l belongs to the session sidebar toggle.
  fireEvent.keyDown(window, { key: "l", shiftKey: true, ...modifier });
  expect(useShortcutStore.getState().bindings.terminal).toBe("mod+shift+j");
  expect(screen.getByLabelText("打开／收起终端的当前快捷键")).toHaveTextContent("按下组合…");
});

test("clearing and restoring defaults act on every row", async () => {
  const user = userEvent.setup();
  renderWithProviders(<ShortcutsSettings />);
  await user.click(screen.getByRole("button", { name: "清除打开设置的快捷键" }));
  expect(screen.getByLabelText("打开设置的当前快捷键")).toHaveTextContent("未设置");
  expect(useShortcutStore.getState().bindings.settings).toBe("");

  await user.click(screen.getByRole("button", { name: "恢复默认" }));
  expect(useShortcutStore.getState().bindings).toEqual(defaultBindings());
  expect(screen.getByLabelText("打开设置的当前快捷键")).toHaveTextContent(bindingLabel("mod+,"));
});

test("Escape cancels capture without changing the binding", async () => {
  const user = userEvent.setup();
  renderWithProviders(<ShortcutsSettings />);
  await user.click(screen.getByRole("button", { name: "重新绑定新建会话" }));
  fireEvent.keyDown(window, { key: "Escape" });
  expect(useShortcutStore.getState().bindings["new-session"]).toBe("mod+n");
  expect(screen.getByLabelText("新建会话的当前快捷键")).toHaveTextContent(bindingLabel("mod+n"));
});
