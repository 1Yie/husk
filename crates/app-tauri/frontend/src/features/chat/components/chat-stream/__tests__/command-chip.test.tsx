// A sent `/command` chip names itself from the same catalog the composer's
// panel lists — one set of descriptions, two surfaces.

import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";

describe("sent command chip", () => {
  test("its tooltip states the catalog's description", async () => {
    const view = applyEvents(emptyView(), [{ UserPrompt: "/compact" }]);
    renderWithProviders(<ChatStream view={view} sessionKey="/project:1" />);
    const chip = screen.getByText("/compact");
    fireEvent.pointerEnter(chip);
    fireEvent.pointerMove(chip);
    expect(await screen.findByText("压缩上下文")).toBeInTheDocument();
  });
});
