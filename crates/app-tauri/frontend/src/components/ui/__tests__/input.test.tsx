import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createRef } from "react";
import { describe, expect, test, vi } from "vitest";
import { Input } from "../input";

describe("Input text assistance", () => {
  test.each([
    ["omitted", {}],
    ["undefined", { autoCorrect: undefined, autoCapitalize: undefined, spellCheck: undefined }],
  ])("disables text assistance when props are %s", (_, props) => {
    render(<Input aria-label="model" {...props} />);
    const field = screen.getByRole("textbox", { name: "model" });
    expect(field).toHaveAttribute("autocorrect", "off");
    expect(field).toHaveAttribute("autocapitalize", "none");
    expect(field).toHaveAttribute("spellcheck", "false");
    expect(field).not.toHaveAttribute("autocomplete");
  });

  test("lets callers explicitly enable text assistance", () => {
    render(<Input aria-label="prose" autoCorrect="on" autoCapitalize="words" spellCheck />);
    const field = screen.getByRole("textbox", { name: "prose" });
    expect(field).toHaveAttribute("autocorrect", "on");
    expect(field).toHaveAttribute("autocapitalize", "words");
    expect(field).toHaveAttribute("spellcheck", "true");
  });

  test("preserves editing, composition events, autocomplete, styling and the field ref", async () => {
    const user = userEvent.setup();
    const ref = createRef<HTMLInputElement>();
    const onChange = vi.fn();
    const onPaste = vi.fn();
    const onComposition = vi.fn();
    const onKeyDown = vi.fn();
    render(
      <Input
        ref={ref} aria-label="model" type="text" defaultValue="model-"
        className="custom-input" autoComplete="on"
        onChange={onChange} onPaste={onPaste} onKeyDown={onKeyDown}
        onCompositionStart={onComposition} onCompositionUpdate={onComposition}
        onCompositionEnd={onComposition}
      />,
    );
    const field = screen.getByRole("textbox", { name: "model" });
    expect(ref.current).toBe(field);
    expect(field).toHaveAttribute("type", "text");
    expect(field).toHaveAttribute("autocomplete", "on");
    expect(field).toHaveClass("custom-input");
    await user.type(field, "raw");
    await user.paste(":32b");
    expect(field).toHaveValue("model-raw:32b");
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
