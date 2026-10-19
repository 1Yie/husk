import { render, screen } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { Textarea } from "../textarea";

describe("Textarea shell", () => {
  /** The border and radius live on a shell the field is clipped by, so the
   *  field's own scrollbar can never ride the border or the rounded corner. */
  test("box chrome puts the scroll on a field inside a clipping shell", () => {
    render(<Textarea aria-label="prompt" defaultValue="hello" />);
    const field = screen.getByRole("textbox", { name: "prompt" });
    const shell = field.parentElement!;
    expect(shell.className).toContain("overflow-hidden");
    expect(shell.className).toContain("rounded-xl");
    expect(field.className).not.toContain("border");
    expect(field.className).toContain("resize-none");
  });

  test("chrome none renders the bare field for overlaid editors", () => {
    render(<Textarea aria-label="composer" chrome="none" />);
    const field = screen.getByRole("textbox", { name: "composer" });
    expect(field.parentElement?.className ?? "").not.toContain("overflow-hidden");
    expect(field.className).toContain("bg-transparent");
  });
});
