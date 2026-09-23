import React from "react";
import { ChevronDown } from "@keyline-icons/react";
import { renderWithTwemoji } from "@/lib/twemoji";

/**
 * Props sized for a DOM element.
 *
 * Two things every renderer here has to survive:
 *   * streamdown runs the JSX runtime with `passNode`, so each component gets
 *     the hast `node` — spreading it paints `node="[object Object]"` into the
 *     DOM (23 times in one short answer);
 *   * the pipeline attaches its own classes (`contains-task-list`,
 *     `task-list-item`, …). A plain `className="…"` followed by `{...props}`
 *     silently loses ours to theirs — which is exactly how task lists ended up
 *     with no list styling at all.
 *
 * So: drop `node`, merge classes with ours first and the pipeline's second
 * (plain concatenation — `cn()`/tailwind-merge is deliberately NOT used here:
 * Tailwind's `text-<size>` also sets line-height, so twMerge silently drops a
 * `leading-*` that precedes it, and these strings are full of that pair).
 */
function domProps<T extends { node?: unknown; className?: string }>(
  props: T,
  ours: string,
): Omit<T, "node" | "className"> & { className: string } {
  const { node: _node, className, ...rest } = props;
  return { ...rest, className: [ours, className].filter(Boolean).join(" ") };
}

/** GFM task lists come as `<ul class="contains-task-list">`; the bullet has to
 *  step aside or every row shows a dot next to its checkbox. */
function listClasses(className: string | undefined, base: string): string {
  const markers = className?.includes("contains-task-list") ? "pl-1 list-none" : "pl-5 list-disc";
  // Ours last so `pl-*`/`list-*` win; the pipeline's own class is appended by
  // `domProps`, never here.
  return [base, markers].filter(Boolean).join(" ");
}

/**
 * Drop the streaming fade's wrappers inside code.
 *
 * The fade splits text into `<span data-sd-animate>` nodes that start at
 * opacity 0 and resolve on a stagger, which is what makes prose arrive smoothly.
 * Code is drawn as a styled element — inline code is a pill with a background
 * and border — so an animated pill paints its box immediately and sits there
 * EMPTY while the text fades in a beat later. Fenced code is spared by the
 * plugin itself (it skips `pre`); inline `code` is not, so the wrappers are
 * unwrapped here. Code is read, not watched.
 */
function unfaded(children: React.ReactNode): React.ReactNode {
  return React.Children.map(children, (child) => {
    if (!React.isValidElement(child)) return child;
    const props = child.props as { "data-sd-animate"?: unknown; children?: React.ReactNode };
    if ("data-sd-animate" in props) return unfaded(props.children);
    return child;
  });
}

export const chatMarkdownComponents = {
  // Same scroll contract as the tool-output card: card shell + plain
  // overflow-x/y-auto inner + the global 6px scrollbar — no bespoke
  // scrollbar class, no wheel remapping.
  table: ({ children, ...props }: React.ComponentPropsWithoutRef<"table">) => (
    <div className="my-4 w-full rounded-xl border border-neutral-200 bg-neutral-50 overflow-hidden shadow-2xs">
      <div className="w-full overflow-x-auto overflow-y-auto max-h-[400px] select-text">
        {/* `w-full` only — cells wrap, so the table fits the card and only
            overflows (→ inner x-scroll) when a single unbreakable token is
            wider than a column. `min-w-max` here would force every table to
            its fully-unwrapped width, guaranteeing overflow on any chat-
            width table. */}
        <table {...domProps(props, "w-full text-left text-[13px] border-collapse")}>
          {children}
        </table>
      </div>
    </div>
  ),
  thead: ({ children, ...props }: React.ComponentPropsWithoutRef<"thead">) => (
    // No sticky here — every sticky element inside the scroll tree is a
    // per-frame layout recalc on WebKitGTK; a table's header can just
    // scroll away with its body.
    <thead {...domProps(props, "bg-neutral-50 border-b border-neutral-200 select-none")}>
      {children}
    </thead>
  ),
  th: ({ children, ...props }: React.ComponentPropsWithoutRef<"th">) => (
    <th
      {...domProps(
        props,
        "px-4 py-2.5 text-left font-semibold text-neutral-800 text-xs tracking-wider uppercase whitespace-nowrap",
      )}
    >
      {renderWithTwemoji(children)}
    </th>
  ),
  tbody: ({ children, ...props }: React.ComponentPropsWithoutRef<"tbody">) => (
    <tbody {...domProps(props, "divide-y divide-neutral-100")}>
      {children}
    </tbody>
  ),
  tr: ({ children, ...props }: React.ComponentPropsWithoutRef<"tr">) => (
    <tr {...domProps(props, "hover:bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] transition-colors")}>
      {children}
    </tr>
  ),
  td: ({ children, ...props }: React.ComponentPropsWithoutRef<"td">) => (
    <td
      {...domProps(
        props,
        "px-4 py-2.5 text-neutral-700 leading-relaxed border-b border-neutral-100 last:border-b-0 align-top break-words",
      )}
    >
      {renderWithTwemoji(children)}
    </td>
  ),

  img: ({ src, alt, ...props }: React.ComponentPropsWithoutRef<"img">) => (
    <figure className="my-4 flex flex-col items-center max-w-full">
      <img
        src={src}
        alt={alt}
        loading="lazy"
        onClick={() => {
          if (src) window.open(src, "_blank");
        }}
        {...domProps(
          props,
          "max-w-full h-auto rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] shadow-xs object-contain transition-all hover:shadow-md cursor-zoom-in",
        )}
      />
      {alt && (
        <figcaption className="mt-2 text-xs text-neutral-500 font-sans text-center">
          {renderWithTwemoji(alt)}
        </figcaption>
      )}
    </figure>
  ),

  blockquote: ({ children, ...props }: React.ComponentPropsWithoutRef<"blockquote">) => (
    <blockquote
      {...domProps(
        props,
        "my-3 border-l-2 border-neutral-300 bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] rounded-r-lg px-4 py-2 text-neutral-600 text-[14px] italic leading-relaxed",
      )}
    >
      {renderWithTwemoji(children)}
    </blockquote>
  ),

  inlineCode: ({ children, ...props }: React.ComponentPropsWithoutRef<"code">) => (
    <code
      {...domProps(
        props,
        "bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] text-neutral-800 px-1.5 py-[1px] rounded-[4px] text-[12px] font-mono border border-[color-mix(in_srgb,var(--husk-black)_7%,transparent)] font-normal mx-0.5 inline-block leading-snug align-baseline",
      )}
    >
      {unfaded(children)}
    </code>
  ),

  pre: ({ children }: React.ComponentPropsWithoutRef<"pre">) => {
    if (React.isValidElement(children)) {
      const className = (children.props as { className?: string }).className || "";
      const hasLang = /language-[^\s]+/.test(className);
      return React.cloneElement(children as React.ReactElement<any>, {
        "data-block": "true",
        className: hasLang ? className : `${className} language-text`.trim(),
      });
    }
    return children;
  },

  a: ({ href, children, ...props }: React.ComponentPropsWithoutRef<"a">) => (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      {...domProps(
        props,
        "text-blue-600 hover:text-blue-700 dark:text-blue-400 dark:hover:text-blue-300 underline underline-offset-2 font-medium transition-colors cursor-pointer",
      )}
    >
      {renderWithTwemoji(children)}
    </a>
  ),

  ul: ({ children, ...props }: React.ComponentPropsWithoutRef<"ul">) => (
    <ul
      {...domProps(
        props,
        listClasses(props.className, "my-2 space-y-1 text-neutral-800 text-[14px] leading-relaxed"),
      )}
    >
      {children}
    </ul>
  ),
  ol: ({ children, ...props }: React.ComponentPropsWithoutRef<"ol">) => (
    <ol {...domProps(props, "my-2 pl-5 list-decimal space-y-1 text-neutral-800 text-[14px] leading-relaxed")}>
      {children}
    </ol>
  ),
  li: ({ children, ...props }: React.ComponentPropsWithoutRef<"li">) => (
    <li {...domProps(props, "leading-relaxed")}>
      {renderWithTwemoji(children)}
    </li>
  ),

  /** Task-list checkboxes are `disabled` inputs the markdown pipeline injects
   *  (nothing in the fence controls them) — give them an accent colour and a
   *  baseline that lines up with the 14px prose instead of the browser default.
   *  Every other input passes through untouched. */
  input: ({ type, ...props }: React.ComponentPropsWithoutRef<"input">) =>
    type === "checkbox" ? (
      <input type="checkbox" {...domProps(props, "mr-1.5 size-3.5 translate-y-[1px] accent-emerald-600 cursor-default")} />
    ) : (
      <input type={type} {...domProps(props, "")} />
    ),

  h1: ({ children, ...props }: React.ComponentPropsWithoutRef<"h1">) => (
    <h1
      {...domProps(
        props,
        "text-xl font-bold text-neutral-900 mt-6 mb-3 tracking-tight border-b border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] pb-2 leading-tight",
      )}
    >
      {renderWithTwemoji(children)}
    </h1>
  ),
  h2: ({ children, ...props }: React.ComponentPropsWithoutRef<"h2">) => (
    <h2
      {...domProps(
        props,
        "text-lg font-bold text-neutral-900 mt-5 mb-2.5 tracking-tight border-b border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] pb-1.5 leading-snug",
      )}
    >
      {renderWithTwemoji(children)}
    </h2>
  ),
  h3: ({ children, ...props }: React.ComponentPropsWithoutRef<"h3">) => (
    <h3
      {...domProps(props, "text-base font-semibold text-neutral-800 mt-4 mb-2 tracking-tight leading-snug")}
    >
      {renderWithTwemoji(children)}
    </h3>
  ),
  h4: ({ children, ...props }: React.ComponentPropsWithoutRef<"h4">) => (
    <h4
      {...domProps(props, "text-[14px] font-semibold text-neutral-800 mt-3 mb-1 tracking-tight")}
    >
      {renderWithTwemoji(children)}
    </h4>
  ),
  p: ({ children, ...props }: React.ComponentPropsWithoutRef<"p">) => (
    <p {...domProps(props, "my-2 leading-relaxed text-[14px] text-neutral-800")}>
      {renderWithTwemoji(children)}
    </p>
  ),
  hr: (props: React.ComponentPropsWithoutRef<"hr">) => (
    <hr {...domProps(props, "my-6 border-0 border-t border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)]")} />
  ),

  details: ({ children, ...props }: React.ComponentPropsWithoutRef<"details">) => (
    <details
      {...domProps(
        props,
        "group my-2 rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] bg-[color-mix(in_srgb,var(--husk-n50)_60%,transparent)] p-2.5 transition-all text-sm leading-relaxed",
      )}
    >
      {renderWithTwemoji(children)}
    </details>
  ),
  summary: ({ children, ...props }: React.ComponentPropsWithoutRef<"summary">) => (
    <summary
      {...domProps(
        props,
        "flex cursor-pointer items-center gap-2 font-medium text-neutral-800 select-none list-none [&::-webkit-details-marker]:hidden",
      )}
    >
      <ChevronDown className="h-3.5 w-3.5 text-neutral-500 transition-transform duration-250 ease-out group-open:rotate-0 -rotate-90 shrink-0" />
      <span>{renderWithTwemoji(children)}</span>
    </summary>
  ),
};

/** Reasoning-panel variant: identical block machinery (fenced code, tables,
 *  images keep their real renderers) with secondary typography — expanded
 *  thinking is a recap of how the answer was reached, so it reads smaller and
 *  dimmer than the answer itself instead of competing with it. */
export const thinkingMarkdownComponents = {
  ...chatMarkdownComponents,

  p: ({ children, ...props }: React.ComponentPropsWithoutRef<"p">) => (
    <p {...domProps(props, "my-2 leading-relaxed text-[13px] text-neutral-500")}>
      {renderWithTwemoji(children)}
    </p>
  ),

  ul: ({ children, ...props }: React.ComponentPropsWithoutRef<"ul">) => (
    <ul
      {...domProps(
        props,
        listClasses(props.className, "my-2 space-y-0.5 text-neutral-500 text-[13px] leading-relaxed"),
      )}
    >
      {children}
    </ul>
  ),
  ol: ({ children, ...props }: React.ComponentPropsWithoutRef<"ol">) => (
    <ol {...domProps(props, "my-2 pl-5 list-decimal space-y-0.5 text-neutral-500 text-[13px] leading-relaxed")}>
      {children}
    </ol>
  ),

  h1: ({ children, ...props }: React.ComponentPropsWithoutRef<"h1">) => (
    <h1 {...domProps(props, "text-[15px] font-semibold text-neutral-700 mt-4 mb-2 tracking-tight")}>
      {renderWithTwemoji(children)}
    </h1>
  ),
  h2: ({ children, ...props }: React.ComponentPropsWithoutRef<"h2">) => (
    <h2 {...domProps(props, "text-[14px] font-semibold text-neutral-700 mt-3.5 mb-1.5 tracking-tight")}>
      {renderWithTwemoji(children)}
    </h2>
  ),
  h3: ({ children, ...props }: React.ComponentPropsWithoutRef<"h3">) => (
    <h3 {...domProps(props, "text-[13px] font-semibold text-neutral-700 mt-3 mb-1 tracking-tight")}>
      {renderWithTwemoji(children)}
    </h3>
  ),
  h4: ({ children, ...props }: React.ComponentPropsWithoutRef<"h4">) => (
    <h4 {...domProps(props, "text-[13px] font-semibold text-neutral-600 mt-2.5 mb-1 tracking-tight")}>
      {renderWithTwemoji(children)}
    </h4>
  ),

  inlineCode: ({ children, ...props }: React.ComponentPropsWithoutRef<"code">) => (
    <code
      {...domProps(
        props,
        "bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] text-neutral-600 px-1.5 py-[1px] rounded-[4px] text-[11.5px] font-mono border border-[color-mix(in_srgb,var(--husk-black)_7%,transparent)] font-normal mx-0.5 inline-block leading-snug align-baseline",
      )}
    >
      {unfaded(children)}
    </code>
  ),

  blockquote: ({ children, ...props }: React.ComponentPropsWithoutRef<"blockquote">) => (
    <blockquote
      {...domProps(
        props,
        "my-2.5 border-l-2 border-neutral-300 bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] rounded-r-lg px-3.5 py-1.5 text-neutral-500 text-[13px] italic leading-relaxed",
      )}
    >
      {renderWithTwemoji(children)}
    </blockquote>
  ),
};
