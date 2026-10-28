import { act, renderHook } from "@testing-library/react";
import { expect, test } from "vitest";
import { notificationHeadline, setSessionTitles, useSessionTitles } from "../notifications";

test("a partial sidebar page does not erase a previously known notification title or conflate roots", () => {
  setSessionTitles([{ root: "/a", sessions: [{ id: 1, title: "older" }] }]);
  const { result } = renderHook(() => useSessionTitles());
  act(() => setSessionTitles([{ root: "/b", sessions: [{ id: 1, title: "other root" }] }], true));
  expect(notificationHeadline({ root: "/a", session: 1, title: "finished" }, result.current)).toContain("older");
  expect(notificationHeadline({ root: "/b", session: 1, title: "finished" }, result.current)).toContain("other root");
  act(() => setSessionTitles([{ root: "/a", sessions: [{ id: 1, title: "renamed" }] }], true));
  expect(notificationHeadline({ root: "/a", session: 1, title: "finished" }, result.current)).toContain("renamed");
  act(() => setSessionTitles([]));
});
