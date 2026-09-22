// Thinking body — plain light muted text under a timeline title row.
// No card background, just clean muted text.

interface Props {
  text: string;
}

export function ThinkingBlock({ text }: Props) {
  return (
    <div className="text-neutral-500 text-[13px] leading-relaxed whitespace-pre-wrap select-text py-0.5">
      {text}
    </div>
  );
}
