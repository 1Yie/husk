import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createRef } from "react";
import { describe, expect, test, vi } from "vitest";
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

describe.each(["box", "none"] as const)("Textarea %s text assistance", (chrome) => {
  test.each([
    ["omitted", {}],
    ["undefined", { autoCorrect: undefined, autoCapitalize: undefined, spellCheck: undefined }],
  ])("disables text assistance on the field when props are %s", (_, props) => {
    render(<Textarea aria-label="parameters" chrome={chrome} {...props} />);
    const field = screen.getByRole("textbox", { name: "parameters" });
    expect(field).toHaveAttribute("autocorrect", "off");
    expect(field).toHaveAttribute("autocapitalize", "none");
    expect(field).toHaveAttribute("spellcheck", "false");
    expect(field).not.toHaveAttribute("autocomplete");
  });

  test("lets callers explicitly enable text assistance", () => {
    render(
      <Textarea aria-label="prose" chrome={chrome}
        autoCorrect="on" autoCapitalize="words" spellCheck />,
    );
    const field = screen.getByRole("textbox", { name: "prose" });
    expect(field).toHaveAttribute("autocorrect", "on");
    expect(field).toHaveAttribute("autocapitalize", "words");
    expect(field).toHaveAttribute("spellcheck", "true");
  });

  test("preserves editing, composition events, autocomplete, styling and the field ref", async () => {
    const user = userEvent.setup();
    const ref = createRef<HTMLTextAreaElement>();
    const onChange = vi.fn();
    const onPaste = vi.fn();
    const onComposition = vi.fn();
    const onKeyDown = vi.fn();
    render(
      <Textarea
        ref={ref} aria-label="parameters" chrome={chrome} defaultValue="model-"
        className="custom-textarea" autoComplete="on"
        onChange={onChange} onPaste={onPaste} onKeyDown={onKeyDown}
        onCompositionStart={onComposition} onCompositionUpdate={onComposition}
        onCompositionEnd={onComposition}
      />,
    );
    const field = screen.getByRole("textbox", { name: "parameters" });
    expect(ref.current).toBe(field);
    expect(field).toHaveAttribute("autocomplete", "on");
    expect(field).toHaveClass("custom-textarea");
    await user.type(field, "raw");
    await user.paste(":32b\nnext");
    expect(field).toHaveValue("model-raw:32b\nnext");
    expect(onChange).toHaveBeenCalled();
    expect(onPaste).toHaveBeenCalledOnce();
    fireEvent.compositionStart(field, { data: "" });
    fireEvent.compositionUpdate(field, { data: "模型" });
    expect(fireEvent.keyDown(field, { key: "Enter", isComposing: true })).toBe(true);
    expect(onKeyDown).toHaveBeenLastCalledWith(expect.objectContaining({
      key: "Enter", nativeEvent: expect.objectContaining({ isComposing: true }),
    }));
    fireEvent.compositionEnd(field, { data: "模型" });
    expect(onComposition.mock.calls.map(([event]) => [event.type, event.data])).toEqual([
      ["compositionstart", ""], ["compositionupdate", "模型"], ["compositionend", "模型"],
    ]);
  });
});
