// Only the desktop boundary is stubbed. The fixture renders the real
// ChatStream, markdown, collapsed reasoning and tool-call components.
export const invoke = async () => ({});
export class Channel { onmessage = () => {}; }
export const convertFileSrc = (src: string) => src;
export const listen = async () => () => {};
export const emit = async () => {};
export const getCurrentWindow = () => ({
  onFocusChanged: async () => () => {},
  isFocused: async () => true,
  startDragging: async () => {},
  toggleMaximize: async () => {},
  isMaximized: async () => false,
  onResized: async () => () => {},
  minimize: async () => {},
  close: async () => {},
});
