const filename = "a-very-long-file-name-for-testing-the-shared-toolbar.ts";
export class Channel { onmessage = (_event: unknown) => {}; }
export async function invoke(command: string, args?: { path?: string }) {
  if (command === "workspace_list_files") {
    return {
      root: "/fixture/workspace", path: args?.path ?? "", truncated: false,
      entries: args?.path === "src"
        ? [{ name: filename, path: `src/${filename}`, kind: "file", size: 24 }]
        : [{ name: "src", path: "src", kind: "directory", size: 0 }],
    };
  }
  return {
    root: "/fixture/workspace", path: `src/${filename}`, name: filename,
    kind: "text", text: "export const value = 1;\n", size: 24, truncated: false,
  };
}
export const convertFileSrc = (path: string) => path;
export const listen = async () => () => {};
export const emit = async () => {};
export const getCurrentWindow = () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true });
