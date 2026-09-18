/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // Codex Desktop light theme tokens matching screenshot.
        workspace: "#ffffff",
        panel: "#f3f3f5",
        card: "#ffffff",
        hover: "#e8e8ed",
        active: "#ffffff",
        hairline: "#e4e4e7",
        focus: "#a1a1aa",
        "text-primary": "#18181b",
        "text-secondary": "#52525b",
        "text-muted": "#71717a",
        "text-dim": "#a1a1aa",
        "bubble-user": "#e3e3e8",
        accent: "#18181b",
        "diff-add": "#16a34a",
        "diff-add-surface": "#f0fdf4",
        "diff-del": "#dc2626",
        "diff-del-surface": "#fef2f2",
        running: "#9333ea",
        warn: "#d97706",
        error: "#dc2626",
      },
      borderRadius: {
        sm: "6px",
        md: "10px",
        lg: "14px",
        xl: "18px",
        "2xl": "22px",
      },
      fontFamily: {
        sans: ['"Inter"', "-apple-system", "BlinkMacSystemFont", "system-ui", "sans-serif"],
        mono: ['"JetBrains Mono"', "ui-monospace", "monospace"],
      },
      boxShadow: {
        "sidebar-active": "0 1px 2px 0 rgba(0, 0, 0, 0.05)",
        "composer": "0 4px 20px 0 rgba(0, 0, 0, 0.06)",
      }
    },
  },
  plugins: [require("tailwindcss-animate")],
};

