import { render } from "@testing-library/react";
import { useRef } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useSidebarGlass } from "./sidebar-glass";

// A platform without the framed shell (Linux) must stay solid without asking
// the backend, so an unsupported machine never reaches the command at all.
const boundary = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
vi.mock("@tauri-apps/api/core", () => boundary);
vi.mock("@/lib/platform", () => ({ framedShell: false }));

let frames: FrameRequestCallback[];
function Fixture() {
  const ref = useRef<HTMLDivElement>(null);
  useSidebarGlass(ref);
  return <div ref={ref} data-testid="shell"><div data-testid="navigation" style={{ width: 240 }} /></div>;
}

beforeEach(() => {
  frames = [];
  boundary.invoke.mockReset();
  boundary.isTauri.mockReturnValue(true);
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback) => { frames.push(callback); return frames.length; }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
});

it("never measures or invokes the backdrop on a platform without the framed shell", async () => {
  const view = render(<Fixture />);
  await Promise.resolve();
  expect(frames).toHaveLength(0);
  expect(boundary.invoke).not.toHaveBeenCalled();
  view.unmount();
  await Promise.resolve();
  expect(boundary.invoke).not.toHaveBeenCalled();
});
