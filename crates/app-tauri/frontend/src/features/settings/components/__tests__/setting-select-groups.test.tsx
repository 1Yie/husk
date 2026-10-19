import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import { SettingSelect } from "..";

/** Provider sections come from a HashMap upstream, so the control — not the
 *  caller — has to put them in a stable alphabetical order. */
describe("SettingSelect groups", () => {
  test("sections are alphabetical while ungrouped options stay on top", async () => {
    const user = userEvent.setup();
    render(
      <SettingSelect
        value=""
        onChange={() => {}}
        ariaLabel="model"
        options={[
          { value: "", label: "跟随默认" },
          { value: "z/1", label: "zeta one", group: "zeta" },
          { value: "a/1", label: "alpha one", group: "alpha" },
          { value: "m/1", label: "mid one", group: "mid" },
        ]}
      />,
    );
    await user.click(screen.getByRole("button", { name: "model" }));
    const labels = (await screen.findAllByRole("menuitemradio")).map(
      (el) => el.textContent ?? "",
    );
    expect(labels.map((l) => l.split(" ")[0])).toEqual([
      "跟随默认",
      "alpha",
      "mid",
      "zeta",
    ]);
  });
});
