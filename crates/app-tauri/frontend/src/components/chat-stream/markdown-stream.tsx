import { memo, useMemo } from "react";
import { cjk } from "@streamdown/cjk";
import { Streamdown } from "streamdown";
import { Check, Copy } from "@keyline-icons/react";
import { chatMarkdownComponents } from "./markdown-components";
import { codeHighlightPlugin, codeHighlightPluginStreaming } from "../../lib/syntax-highlight";

/** Streamdown's toolbar icons — module-level so the renderer's `icons` prop
 *  stays referentially stable (a fresh object defeats Streamdown's own
 *  per-block memo). */
export const streamdownIcons = {
  CheckIcon: Check,
  CopyIcon: Copy,
};

/** Streamdown in `memo`: a finished block's `text` reference is stable, so only
 * the block whose text changed re-renders. `plugins` is memoized on `animating` — a
 * fresh object would defeat Streamdown's own per-block memo (its comparator includes
 * `plugins`) and re-render every code block; while streaming it also selects the
 * complete-lines-only highlighter.
 *
 * Shared by the answer text, the reasoning panel and tool prose — the caller picks
 * the block styling through `components`. */
export const MemoStreamdown = memo(function MemoStreamdown({
  text,
  animating,
  components = chatMarkdownComponents,
}: {
  text: string;
  animating?: boolean;
  /** Block styling map. Defaults to the answer's; the reasoning panel passes
   *  a muted variant so expanded thinking doesn't read as answer text. */
  components?: typeof chatMarkdownComponents;
}) {
  const plugins = useMemo(
    () => ({ cjk, code: (animating ? codeHighlightPluginStreaming : codeHighlightPlugin) as any }),
    [animating],
  );
  return (
    <Streamdown
      isAnimating={animating}
      plugins={plugins}
      shikiTheme={["github-dark", "github-dark"]}
      components={components}
      icons={streamdownIcons}
    >
      {text}
    </Streamdown>
  );
});
