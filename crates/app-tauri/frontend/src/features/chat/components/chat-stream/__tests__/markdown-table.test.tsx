// Table columns keep their own width: the table is as wide as its content and
// the card's scroller moves it. Cells wrap at word boundaries, not per glyph.

import { render } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { chatMarkdownComponents } from "@/features/chat/components/chat-stream/markdown-components";

const { table: Table, th: Th, td: Td } = chatMarkdownComponents;

function classes(el: Element): string[] {
  return el.className.split(/\s+/).filter(Boolean);
}

describe("markdown table", () => {
  test("sizes to its content and can overflow the card", () => {
    const { container } = render(<Table><tbody /></Table>);
    const table = container.querySelector("table") as HTMLElement;
    const list = classes(table);
    expect(list).toContain("w-max");
    expect(list).toContain("min-w-full");
    expect(list).not.toContain("w-full");
  });

  test("cells wrap at word boundaries within a sane width", () => {
    const { container } = render(
      <table>
        <tbody>
          <tr>
            <Td>一段表格内容</Td>
          </tr>
        </tbody>
      </table>,
    );
    const cell = container.querySelector("td") as HTMLElement;
    const list = classes(cell);
    expect(list).not.toContain("break-words");
    expect(list).not.toContain("break-all");
    expect(list).toContain("[overflow-wrap:normal]");
    expect(list).toContain("[word-break:normal]");
    expect(list).toContain("min-w-[6em]");
    expect(list).toContain("max-w-[32rem]");
  });

  test("headers stay on one line, unwrapped mid-word", () => {
    const { container } = render(
      <table>
        <thead>
          <tr>
            <Th>名称</Th>
          </tr>
        </thead>
      </table>,
    );
    const head = container.querySelector("th") as HTMLElement;
    const list = classes(head);
    expect(list).toContain("whitespace-nowrap");
    expect(list).toContain("[overflow-wrap:normal]");
    expect(list).toContain("[word-break:normal]");
  });

  test("keeps the column alignment markdown asked for", () => {
    const { container } = render(
      <table>
        <tbody>
          <tr>
            <Td align="right" style={{ textAlign: "right" }}>12</Td>
          </tr>
        </tbody>
      </table>,
    );
    const cell = container.querySelector("td") as HTMLElement;
    expect(cell.getAttribute("align")).toBe("right");
    expect(cell).toHaveStyle({ textAlign: "right" });
  });
});
