/** @type {import('tailwindcss').Config} */
/** A theme colour that still takes a Tailwind alpha modifier. Tailwind cannot
 *  build `/80` out of a bare `var()`, so the value folds through `color-mix`
 *  with the substituted `<alpha-value>` (100% when no modifier is written). */
const alpha = (varName) =>
  `color-mix(in srgb, ${varName} calc(<alpha-value> * 100%), transparent)`;

export default {
  darkMode: "class",
  content: [
    "./index.html",
    "./src/**/*.{ts,tsx}",
    // streamdown draws its own chrome (mermaid cards, diagram controls, the
    // fullscreen overlay, table menus) with utility classes that live in the
    // package, not in this repo — unscanned, Tailwind emits no rule for them and
    // those blocks render unstyled however the theme is configured.
    "./node_modules/streamdown/dist/*.js",
    "./node_modules/@streamdown/*/dist/*.js",
  ],
  theme: {
    extend: {
      colors: {
        // Theme-driven tokens — CSS vars live in index.css (`:root` light,
        // `.dark` dark). `appearance.json` can override them at runtime.
        workspace: "var(--husk-bg)",
        panel: "var(--husk-panel)",
        card: "var(--husk-card)",
        hover: "var(--husk-hover)",
        active: "var(--husk-active)",
        hairline: "var(--husk-border)",
        focus: "var(--husk-focus)",
        "text-primary": "var(--husk-fg)",
        "text-secondary": "var(--husk-fg-secondary)",
        "text-muted": "var(--husk-fg-muted)",
        "text-dim": "var(--husk-fg-dim)",
        "bubble-user": "var(--husk-bubble-user)",
        accent: "var(--husk-accent)",

        // shadcn-style names that streamdown's OWN chrome is written against
        // (`border-border`, `bg-sidebar`, `bg-background/95`,
        // `text-muted-foreground`, `bg-primary`, …). The app palette above uses
        // its own names, so without these Tailwind emits no rule at all and
        // everything streamdown draws itself — mermaid cards, diagram controls,
        // the fullscreen overlay, table chrome — renders unstyled. The overlay
        // was the visible one: `bg-background/95` painted nothing, so fullscreen
        // showed the diagram floating over an undimmed page.
        background: alpha("var(--husk-bg)"),
        foreground: "var(--husk-fg)",
        border: alpha("var(--husk-border)"),
        muted: {
          DEFAULT: alpha("var(--husk-n100)"),
          foreground: "var(--husk-fg-muted)",
        },
        sidebar: {
          DEFAULT: alpha("var(--husk-panel)"),
          foreground: "var(--husk-fg)",
        },
        primary: {
          DEFAULT: alpha("var(--husk-n900)"),
          foreground: "var(--husk-white)",
        },
        "diff-add": "#16a34a",
        "diff-add-surface": "#f0fdf4",
        "diff-del": "#dc2626",
        "diff-del-surface": "#fef2f2",
        running: "#9333ea",
        warn: "#d97706",
        error: "#dc2626",
        // Literal white/black — used for text on colored surfaces and for
        // card backgrounds. In dark mode they invert so `bg-white` (a card
        // in light) becomes the dark card tone and `text-white` stays
        // readable on dark backgrounds.
        white: "var(--husk-white)",
        black: "var(--husk-black)",
        // Neutral scale → CSS vars so `bg-neutral-100`/`text-neutral-600` etc.
        // follow the theme instead of pinning literal zinc values.
        neutral: {
          50: "var(--husk-n50)",
          100: "var(--husk-n100)",
          200: "var(--husk-n200)",
          300: "var(--husk-n300)",
          400: "var(--husk-n400)",
          500: "var(--husk-n500)",
          600: "var(--husk-n600)",
          700: "var(--husk-n700)",
          800: "var(--husk-n800)",
          900: "var(--husk-n900)",
          950: "var(--husk-n950)",
        },
      },
      borderRadius: {
        sm: "6px",
        md: "10px",
        lg: "14px",
        xl: "18px",
        "2xl": "22px",
      },
      fontFamily: {
        sans: ['"MiSans"', '"Inter"', "-apple-system", "BlinkMacSystemFont", "system-ui", "sans-serif"],
        mono: ['"JetBrains Mono"', '"JetBrainsMono Nerd Font Mono"', '"JetBrainsMono NFM"', "ui-monospace", "monospace"],
      },
      boxShadow: {
        "sidebar-active": "0 1px 2px 0 rgba(0, 0, 0, 0.05)",
        "composer": "0 4px 20px 0 rgba(0, 0, 0, 0.06)",
        /* Elevation for every popup layer (menus, popover, dialog, popup cards).
           Tailwind's shadow-md/-lg keep a hard, near-opaque band right under the
           popup edge; when a scrollbar/rail from the page runs behind the popup,
           that band darkens the top of the bar into a hard grey right angle that
           reads as "a square shadow hanging off the popup's rounded corner".
           A wide blur with negative spread keeps the same elevation while the
           edge falloff stays soft enough not to bite into what is behind. */
        "popup": "0 8px 24px -6px rgba(0, 0, 0, 0.16), 0 2px 6px -2px rgba(0, 0, 0, 0.06)",
      },
      keyframes: {
        "collapsible-down": {
          from: { height: "0", opacity: "0" },
          to: { height: "var(--radix-collapsible-content-height)", opacity: "1" },
        },
        "collapsible-up": {
          from: { height: "var(--radix-collapsible-content-height)", opacity: "1" },
          to: { height: "0", opacity: "0" },
        },
      },
      animation: {
        "collapsible-down": "collapsible-down 0.25s cubic-bezier(0.16, 1, 0.3, 1)",
        "collapsible-up": "collapsible-up 0.2s cubic-bezier(0.16, 1, 0.3, 1)",
      },
    },
  },
  plugins: [require("tailwindcss-animate")],
};

