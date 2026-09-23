import React from "react";
import { ChevronDown } from "@keyline-icons/react";
import { renderWithTwemoji } from "@/lib/twemoji";

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
        <table className="w-full text-left text-[13px] border-collapse" {...props}>
          {children}
        </table>
      </div>
    </div>
  ),
  thead: ({ children, ...props }: React.ComponentPropsWithoutRef<"thead">) => (
    // No sticky here — every sticky element inside the scroll tree is a
    // per-frame layout recalc on WebKitGTK; a table's header can just
    // scroll away with its body.
    <thead
      className="bg-neutral-50 border-b border-neutral-200 select-none"
      {...props}
    >
      {children}
    </thead>
  ),
  th: ({ children, ...props }: React.ComponentPropsWithoutRef<"th">) => (
    <th
      className="px-4 py-2.5 text-left font-semibold text-neutral-800 text-xs tracking-wider uppercase whitespace-nowrap"
      {...props}
    >
      {renderWithTwemoji(children)}
    </th>
  ),
  tbody: ({ children, ...props }: React.ComponentPropsWithoutRef<"tbody">) => (
    <tbody className="divide-y divide-neutral-100" {...props}>
      {children}
    </tbody>
  ),
  tr: ({ children, ...props }: React.ComponentPropsWithoutRef<"tr">) => (
    <tr
      className="hover:bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] transition-colors"
      {...props}
    >
      {children}
    </tr>
  ),
  td: ({ children, ...props }: React.ComponentPropsWithoutRef<"td">) => (
    <td
      className="px-4 py-2.5 text-neutral-700 leading-relaxed border-b border-neutral-100 last:border-b-0 align-top break-words"
      {...props}
    >
      {renderWithTwemoji(children)}
    </td>
  ),

  img: ({ src, alt, ...props }: React.ComponentPropsWithoutRef<"img">) => (
    <figure className="my-4 flex flex-col items-center max-w-full">
      <img
        src={src}
        alt={alt}
        className="max-w-full h-auto rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] shadow-xs object-contain transition-all hover:shadow-md cursor-zoom-in"
        loading="lazy"
        onClick={() => {
          if (src) window.open(src, "_blank");
        }}
        {...props}
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
      className="my-3 border-l-2 border-neutral-300 bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] rounded-r-lg px-4 py-2 text-neutral-600 text-[14px] italic leading-relaxed"
      {...props}
    >
      {renderWithTwemoji(children)}
    </blockquote>
  ),

  inlineCode: ({ children, ...props }: React.ComponentPropsWithoutRef<"code">) => (
    <code
      className="bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] text-neutral-800 px-1.5 py-[1px] rounded-[4px] text-[12px] font-mono border border-[color-mix(in_srgb,var(--husk-black)_7%,transparent)] font-normal mx-0.5 inline-block leading-snug align-baseline"
      {...props}
    >
      {children}
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
      className="text-blue-600 hover:text-blue-700 dark:text-blue-400 dark:hover:text-blue-300 underline underline-offset-2 font-medium transition-colors cursor-pointer"
      {...props}
    >
      {renderWithTwemoji(children)}
    </a>
  ),

  ul: ({ children, ...props }: React.ComponentPropsWithoutRef<"ul">) => (
    <ul
      className="my-2 pl-5 list-disc space-y-1 text-neutral-800 text-[14px] leading-relaxed"
      {...props}
    >
      {children}
    </ul>
  ),
  ol: ({ children, ...props }: React.ComponentPropsWithoutRef<"ol">) => (
    <ol
      className="my-2 pl-5 list-decimal space-y-1 text-neutral-800 text-[14px] leading-relaxed"
      {...props}
    >
      {children}
    </ol>
  ),
  li: ({ children, ...props }: React.ComponentPropsWithoutRef<"li">) => (
    <li className="leading-relaxed" {...props}>
      {renderWithTwemoji(children)}
    </li>
  ),

  h1: ({ children, ...props }: React.ComponentPropsWithoutRef<"h1">) => (
    <h1
      className="text-xl font-bold text-neutral-900 mt-6 mb-3 tracking-tight border-b border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)] pb-2 leading-tight"
      {...props}
    >
      {renderWithTwemoji(children)}
    </h1>
  ),
  h2: ({ children, ...props }: React.ComponentPropsWithoutRef<"h2">) => (
    <h2
      className="text-lg font-bold text-neutral-900 mt-5 mb-2.5 tracking-tight border-b border-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)] pb-1.5 leading-snug"
      {...props}
    >
      {renderWithTwemoji(children)}
    </h2>
  ),
  h3: ({ children, ...props }: React.ComponentPropsWithoutRef<"h3">) => (
    <h3
      className="text-base font-semibold text-neutral-800 mt-4 mb-2 tracking-tight leading-snug"
      {...props}
    >
      {renderWithTwemoji(children)}
    </h3>
  ),
  h4: ({ children, ...props }: React.ComponentPropsWithoutRef<"h4">) => (
    <h4
      className="text-[14px] font-semibold text-neutral-800 mt-3 mb-1 tracking-tight"
      {...props}
    >
      {renderWithTwemoji(children)}
    </h4>
  ),
  p: ({ children, ...props }: React.ComponentPropsWithoutRef<"p">) => (
    <p
      className="my-2 leading-relaxed text-[14px] text-neutral-800"
      {...props}
    >
      {renderWithTwemoji(children)}
    </p>
  ),
  hr: (props: React.ComponentPropsWithoutRef<"hr">) => (
    <hr
      className="my-6 border-0 border-t border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)]"
      {...props}
    />
  ),

  details: ({ children, ...props }: React.ComponentPropsWithoutRef<"details">) => (
    <details
      className="group my-2 rounded-xl border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] bg-[color-mix(in_srgb,var(--husk-n50)_60%,transparent)] p-2.5 transition-all text-sm leading-relaxed"
      {...props}
    >
      {renderWithTwemoji(children)}
    </details>
  ),
  summary: ({ children, ...props }: React.ComponentPropsWithoutRef<"summary">) => (
    <summary
      className="flex cursor-pointer items-center gap-2 font-medium text-neutral-800 select-none list-none [&::-webkit-details-marker]:hidden"
      {...props}
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
    <p className="my-2 leading-relaxed text-[13px] text-neutral-500" {...props}>
      {renderWithTwemoji(children)}
    </p>
  ),

  ul: ({ children, ...props }: React.ComponentPropsWithoutRef<"ul">) => (
    <ul
      className="my-2 pl-5 list-disc space-y-0.5 text-neutral-500 text-[13px] leading-relaxed"
      {...props}
    >
      {children}
    </ul>
  ),
  ol: ({ children, ...props }: React.ComponentPropsWithoutRef<"ol">) => (
    <ol
      className="my-2 pl-5 list-decimal space-y-0.5 text-neutral-500 text-[13px] leading-relaxed"
      {...props}
    >
      {children}
    </ol>
  ),

  h1: ({ children, ...props }: React.ComponentPropsWithoutRef<"h1">) => (
    <h1 className="text-[15px] font-semibold text-neutral-700 mt-4 mb-2 tracking-tight" {...props}>
      {renderWithTwemoji(children)}
    </h1>
  ),
  h2: ({ children, ...props }: React.ComponentPropsWithoutRef<"h2">) => (
    <h2 className="text-[14px] font-semibold text-neutral-700 mt-3.5 mb-1.5 tracking-tight" {...props}>
      {renderWithTwemoji(children)}
    </h2>
  ),
  h3: ({ children, ...props }: React.ComponentPropsWithoutRef<"h3">) => (
    <h3 className="text-[13px] font-semibold text-neutral-700 mt-3 mb-1 tracking-tight" {...props}>
      {renderWithTwemoji(children)}
    </h3>
  ),
  h4: ({ children, ...props }: React.ComponentPropsWithoutRef<"h4">) => (
    <h4 className="text-[13px] font-semibold text-neutral-600 mt-2.5 mb-1 tracking-tight" {...props}>
      {renderWithTwemoji(children)}
    </h4>
  ),

  inlineCode: ({ children, ...props }: React.ComponentPropsWithoutRef<"code">) => (
    <code
      className="bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] text-neutral-600 px-1.5 py-[1px] rounded-[4px] text-[11.5px] font-mono border border-[color-mix(in_srgb,var(--husk-black)_7%,transparent)] font-normal mx-0.5 inline-block leading-snug align-baseline"
      {...props}
    >
      {children}
    </code>
  ),

  blockquote: ({ children, ...props }: React.ComponentPropsWithoutRef<"blockquote">) => (
    <blockquote
      className="my-2.5 border-l-2 border-neutral-300 bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] rounded-r-lg px-3.5 py-1.5 text-neutral-500 text-[13px] italic leading-relaxed"
      {...props}
    >
      {renderWithTwemoji(children)}
    </blockquote>
  ),
};
