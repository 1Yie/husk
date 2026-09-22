import React, { useState } from "react";
import twemoji from "@twemoji/api";

interface TwemojiItemProps {
  match: string;
  codePoint: string;
}

/**
 * Individual Twemoji image with automatic fallback.
 * Falls back to the native unicode emoji when the SVG fails to load (offline or a
 * missing glyph), so no broken '?' icon appears.
 */
export function TwemojiItem({ match, codePoint }: TwemojiItemProps) {
  const [failed, setFailed] = useState(false);

  if (failed || !codePoint) {
    return <span className="inline-block mx-[0.05em] select-text">{match}</span>;
  }

  return (
    <img
      className="twemoji inline-block h-[1.2em] w-[1.2em] align-[-0.2em] mx-[0.05em] select-none pointer-events-none"
      alt={match}
      draggable={false}
      src={`https://cdn.jsdelivr.net/gh/jdecked/twemoji@17.0.3/assets/svg/${codePoint}.svg`}
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}

/**
 * Parses a string or React children to replace unicode emoji characters
 * with Twemoji SVG elements, falling back to native text when an image is unavailable.
 */
export function renderWithTwemoji(node: React.ReactNode): React.ReactNode {
  if (node == null) return node;

  if (typeof node === "string") {
    if (!twemoji.test(node)) {
      return node;
    }

    const parts: React.ReactNode[] = [];
    let lastIndex = 0;

    twemoji.replace(node, (match: string) => {
      const index = node.indexOf(match, lastIndex);
      if (index > lastIndex) {
        parts.push(node.slice(lastIndex, index));
      }

      const codePoint = twemoji.convert.toCodePoint(match);
      parts.push(
        <TwemojiItem
          key={`twemoji-${codePoint}-${parts.length}`}
          match={match}
          codePoint={codePoint}
        />
      );

      lastIndex = index + match.length;
      return match;
    });

    if (lastIndex < node.length) {
      parts.push(node.slice(lastIndex));
    }

    return parts.length > 0 ? parts : node;
  }

  if (Array.isArray(node)) {
    return React.Children.map(node, renderWithTwemoji);
  }

  if (React.isValidElement(node)) {
    // Never parse emojis inside code blocks or pre tags
    if (node.type === "code" || node.type === "pre") {
      return node;
    }
    const props = node.props as { children?: React.ReactNode };
    if (props && props.children) {
      return React.cloneElement(node, {
        ...props,
        children: renderWithTwemoji(props.children),
      });
    }
  }

  return node;
}
