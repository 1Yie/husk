import { render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { ErrorBoundary } from "../error-boundary";

function Boom(): never {
  throw new Error("boom");
}

test("renders a reloadable fallback instead of a blank window", () => {
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  render(
    <ErrorBoundary>
      <Boom />
    </ErrorBoundary>,
  );
  expect(screen.getByRole("alert")).toHaveTextContent("界面渲染出错");
  expect(screen.getByRole("button", { name: "重新加载" })).toBeInTheDocument();
  expect(consoleError).toHaveBeenCalled();
});
