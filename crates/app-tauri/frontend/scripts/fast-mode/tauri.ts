const config = {
  providers: {
    OpenAI: {
      kind: "openai_responses",
      base_url: "https://api.example.invalid/v1",
      models: [{
        id: "gpt-6.1-sol", name: "GPT 6.1 Sol", context_window: 256000,
        reasoning: true, fastMode: { mode: "priority" },
      }],
    },
  },
};

export class Channel { onmessage = (_event: unknown) => {}; }
export async function invoke(command: string, args?: { op?: string }) {
  if (command === "agent_session") {
    if (args?.op === "get_app_config") return { config, path: "/fixture/config.toml" };
    if (args?.op === "get_default_prefs") return { active_provider: "OpenAI", active_model: "gpt-6.1-sol" };
  }
  return {};
}
export const convertFileSrc = (source: string) => source;
export const listen = async () => () => {};
export const emit = async () => {};
export const getCurrentWindow = () => ({
  onFocusChanged: async () => () => {}, isFocused: async () => true,
});
