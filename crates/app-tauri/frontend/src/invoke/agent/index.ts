// Agent invoke layer — the single typed boundary over the Tauri `agent_*`
// IPC. Components and hooks never touch `@tauri-apps/api` directly.
//
//   import * as agent from "../../invoke/agent";
//   await agent.sendPrompt(text);
//   await agent.listSessions();

export * from "./commands";
export * from "./sessions";
export * from "./events";
