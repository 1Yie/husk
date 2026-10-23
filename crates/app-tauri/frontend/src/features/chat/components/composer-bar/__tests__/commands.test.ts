import { describe, expect, test } from "vitest";
import {
  BUILTIN_COMMANDS,
  commandDesc,
  findCommand,
  localCommandOf,
} from "@/features/chat/components/composer-bar/commands";

describe("builtin command catalog", () => {
  test("no command is listed twice", () => {
    const names = BUILTIN_COMMANDS.map((cmd) => cmd.name);
    expect(new Set(names).size).toBe(names.length);
  });

  test("the desktop-local set is exactly the four UI actions", () => {
    expect(BUILTIN_COMMANDS.filter((c) => c.kind === "local").map((c) => c.name)).toEqual([
      "model",
      "diff",
      "skills",
      "memory",
    ]);
  });

  test("every entry carries a description and a known kind", () => {
    for (const cmd of BUILTIN_COMMANDS) {
      expect(cmd.desc.length).toBeGreaterThan(0);
      expect(["send", "local"]).toContain(cmd.kind);
    }
  });

  test("forget is not advertised but still resolves as an unknown command", () => {
    expect(findCommand("forget")).toBeUndefined();
    expect(commandDesc("forget")).toBeUndefined();
  });

  test("a bare local command is recognised for the submit path", () => {
    expect(localCommandOf("/model")?.name).toBe("model");
    expect(localCommandOf("  /diff ")?.name).toBe("diff");
    // Arguments make it text the kernel should see, not a UI action.
    expect(localCommandOf("/model test/model-b")).toBeUndefined();
    // Send commands keep going to the kernel.
    expect(localCommandOf("/clear")).toBeUndefined();
    expect(localCommandOf("普通文本")).toBeUndefined();
  });

  test("descriptions come from the same catalog the panel renders", () => {
    expect(commandDesc("clear")).toBe("清空当前会话");
    expect(commandDesc("remember")).toBe("记住一条偏好");
    expect(findCommand("blueprint")?.args).toBe("[init|check|graph]");
  });
});
