import { render, screen } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { ReasoningEffort } from "../reasoning-effort";
import { thinkingTone } from "@/lib/thinking";

describe("reasoning effort tier colour", () => {
  test("the tier word takes the tone of the stop it sits on", () => {
    const values = ["low", "medium", "max"];
    render(
      <ReasoningEffort
        stops={["Low", "Medium", "Max"]}
        levelTones={values.map(thinkingTone)}
        value={2}
        label="Claude Opus 5.5"
      />,
    );
    expect(screen.getByText("Max")).toHaveStyle({ color: "var(--husk-think-max, var(--husk-fg-muted))" });
    // The model name stays the caption colour — only the tier is toned.
    expect(screen.getByText("Claude Opus 5.5").getAttribute("style")).toBeNull();
  });

  test("without tones the tier keeps the dim caption colour", () => {
    render(<ReasoningEffort stops={["Low", "High"]} value={1} label="m" />);
    expect(screen.getByText("High").getAttribute("style")).toBeNull();
  });

  test("an unknown tier falls back to the muted ink", () => {
    expect(thinkingTone("turbo")).toBe("var(--husk-think-turbo, var(--husk-fg-muted))");
  });
});
