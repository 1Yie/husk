import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(viteConfig, defineConfig({
  test: {
    environment: "jsdom",
    environmentOptions: { jsdom: { url: "http://localhost:1420", pretendToBeVisual: true } },
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    clearMocks: true,
    restoreMocks: true,
    maxWorkers: 2,
    coverage: {
      provider: "v8",
      reporter: ["text", "html", "lcov", "json-summary"],
      reportsDirectory: "./coverage",
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/*.test.{ts,tsx}", "src/test/**", "src/types/**", "src/**/*.d.ts",
        "src/main.tsx", "src/vite-env.d.ts",
      ],
      thresholds: {
        lines: 25, statements: 25, functions: 25, branches: 65,
        "src/stores/agent-store.ts": { lines: 90, statements: 90, functions: 80, branches: 80 },
        "src/features/chat/components/assistant-status/index.tsx": {
          lines: 95, statements: 95, functions: 95, branches: 90,
        },
        "src/features/chat/hooks/{apply-event,view-from-history}.ts": {
          lines: 50, statements: 50, functions: 50, branches: 55,
        },
        "src/lib/thinking.ts": { lines: 100, functions: 100, branches: 100, statements: 100 },
        "src/features/chat/components/chat-stream/rail-history.ts": {
          lines: 100, functions: 100, branches: 95, statements: 100,
        },
        "src/features/chat/components/composer-bar/composer-state.ts": {
          lines: 100, functions: 100, branches: 95, statements: 100,
        },
      },
    },
  },
}));
