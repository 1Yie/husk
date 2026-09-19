import React from "react";
import { ChevronDown } from "@keyline-icons/react";

export const codexMarkdownComponents = {
  table: ({ children, ...props }: React.ComponentPropsWithoutRef<"table">) => (
    <div className="my-4 w-full rounded-lg border border-neutral-200/90 dark:border-neutral-800 bg-white dark:bg-neutral-900 shadow-xs overflow-hidden">
      <div className="overflow-x-auto">
        <table className="min-w-full text-left text-[13px] border-collapse" {...props}>
          {children}
        </table>
      </div>
    </div>
  ),
  thead: ({ children, ...props }: React.ComponentPropsWithoutRef<"thead">) => (
    <thead className="bg-neutral-50/90 dark:bg-neutral-800/60 border-b border-neutral-200 dark:border-neutral-700 select-none" {...props}>
      {children}
    </thead>
  ),
  th: ({ children, ...props }: React.ComponentPropsWithoutRef<"th">) => (
    <th
      className="px-4 py-2.5 text-left font-semibold text-neutral-800 dark:text-neutral-200 text-xs tracking-wider uppercase whitespace-nowrap"
      {...props}
    >
      {children}
    </th>
  ),
  tbody: ({ children, ...props }: React.ComponentPropsWithoutRef<"tbody">) => (
    <tbody className="divide-y divide-neutral-100 dark:divide-neutral-800" {...props}>
      {children}
    </tbody>
  ),
  tr: ({ children, ...props }: React.ComponentPropsWithoutRef<"tr">) => (
    <tr className="hover:bg-neutral-50/70 dark:hover:bg-neutral-800/40 transition-colors" {...props}>
      {children}
    </tr>
  ),
  td: ({ children, ...props }: React.ComponentPropsWithoutRef<"td">) => (
    <td
      className="px-4 py-2.5 text-neutral-700 dark:text-neutral-300 leading-relaxed border-b border-neutral-100 dark:border-neutral-800 last:border-b-0 align-top"
      {...props}
    >
      {children}
    </td>
  ),

  img: ({ src, alt, ...props }: React.ComponentPropsWithoutRef<"img">) => (
    <figure className="my-4 flex flex-col items-center max-w-full">
      <img
        src={src}
        alt={alt}
        className="max-w-full h-auto rounded-xl border border-neutral-200/90 dark:border-neutral-800 shadow-xs object-contain transition-all hover:shadow-md cursor-zoom-in"
        loading="lazy"
        onClick={() => {
          if (src) window.open(src, "_blank");
        }}
        {...props}
      />
      {alt && (
        <figcaption className="mt-2 text-xs text-neutral-500 font-sans text-center">
          {alt}
        </figcaption>
      )}
    </figure>
  ),

  blockquote: ({ children, ...props }: React.ComponentPropsWithoutRef<"blockquote">) => (
    <blockquote
      className="my-3 border-l-2 border-neutral-300 dark:border-neutral-600 bg-neutral-50/70 dark:bg-neutral-900/50 rounded-r-lg px-4 py-2 text-neutral-600 dark:text-neutral-400 text-[14px] italic leading-relaxed"
      {...props}
    >
      {children}
    </blockquote>
  ),

  inlineCode: ({ children, ...props }: React.ComponentPropsWithoutRef<"code">) => (
    <code
      className="bg-black/[0.06] dark:bg-white/[0.1] text-neutral-800 dark:text-neutral-200 px-1.5 py-0.5 rounded text-[12.5px] font-mono border border-black/[0.08] dark:border-white/[0.1] font-medium mx-0.5 inline align-baseline"
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
      {children}
    </a>
  ),

  ul: ({ children, ...props }: React.ComponentPropsWithoutRef<"ul">) => (
    <ul className="my-2 pl-5 list-disc space-y-1 text-neutral-800 dark:text-neutral-200 text-[14px] leading-relaxed" {...props}>
      {children}
    </ul>
  ),
  ol: ({ children, ...props }: React.ComponentPropsWithoutRef<"ol">) => (
    <ol className="my-2 pl-5 list-decimal space-y-1 text-neutral-800 dark:text-neutral-200 text-[14px] leading-relaxed" {...props}>
      {children}
    </ol>
  ),
  li: ({ children, ...props }: React.ComponentPropsWithoutRef<"li">) => (
    <li className="leading-relaxed" {...props}>
      {children}
    </li>
  ),

  h1: ({ children, ...props }: React.ComponentPropsWithoutRef<"h1">) => (
    <h1 className="text-xl font-bold text-neutral-900 dark:text-neutral-100 mt-6 mb-3 tracking-tight border-b border-neutral-200/70 dark:border-neutral-800 pb-2 leading-tight" {...props}>
      {children}
    </h1>
  ),
  h2: ({ children, ...props }: React.ComponentPropsWithoutRef<"h2">) => (
    <h2 className="text-lg font-bold text-neutral-900 dark:text-neutral-100 mt-5 mb-2.5 tracking-tight border-b border-neutral-200/50 dark:border-neutral-800 pb-1.5 leading-snug" {...props}>
      {children}
    </h2>
  ),
  h3: ({ children, ...props }: React.ComponentPropsWithoutRef<"h3">) => (
    <h3 className="text-base font-semibold text-neutral-800 dark:text-neutral-200 mt-4 mb-2 tracking-tight leading-snug" {...props}>
      {children}
    </h3>
  ),
  h4: ({ children, ...props }: React.ComponentPropsWithoutRef<"h4">) => (
    <h4 className="text-[14px] font-semibold text-neutral-800 dark:text-neutral-200 mt-3 mb-1 tracking-tight" {...props}>
      {children}
    </h4>
  ),
  p: ({ children, ...props }: React.ComponentPropsWithoutRef<"p">) => (
    <p className="my-2 leading-relaxed text-[14px] text-neutral-800 dark:text-neutral-200" {...props}>
      {children}
    </p>
  ),
  hr: (props: React.ComponentPropsWithoutRef<"hr">) => (
    <hr className="my-6 border-0 border-t border-neutral-200/80 dark:border-neutral-800" {...props} />
  ),

  details: ({ children, ...props }: React.ComponentPropsWithoutRef<"details">) => (
    <details
      className="group my-2 rounded-xl border border-neutral-200/90 dark:border-neutral-800 bg-neutral-50/60 dark:bg-neutral-900/40 p-2.5 transition-all text-sm leading-relaxed"
      {...props}
    >
      {children}
    </details>
  ),
  summary: ({ children, ...props }: React.ComponentPropsWithoutRef<"summary">) => (
    <summary
      className="flex cursor-pointer items-center gap-2 font-medium text-neutral-800 dark:text-neutral-200 select-none list-none [&::-webkit-details-marker]:hidden"
      {...props}
    >
      <ChevronDown className="h-3.5 w-3.5 text-neutral-400 transition-transform duration-250 ease-out group-open:rotate-0 -rotate-90 shrink-0" />
      <span>{children}</span>
    </summary>
  ),
};
