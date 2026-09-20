import React from "react";
import { createRoot } from "react-dom/client";
import "./src/index.css";
import { MoreHorizontal, GitFork, Bin } from "@keyline-icons/react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SettingSelect } from "@/components/settings";

/* Scratch visual-check page (untracked, next to harness.html):
   `?state=hover` paints the last menu item's hover/`:focus` background without
   needing a focused window, and renders a page scrollbar behind the popup so
   the popup's shadow token can be judged against real background chrome.
   Usage: `vite --config vite.harness.config.ts`, then open /harness.html. */

const q = new URLSearchParams(location.search);
const hover = q.get("state") === "hover";

function SessionMenu() {
  return (
    <DropdownMenu open>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="absolute inset-0 rounded flex items-center justify-center text-neutral-400"
        >
          <MoreHorizontal className="h-3.5 w-3.5" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="right" align="start" sideOffset={4} className="min-w-[140px]">
        <DropdownMenuItem className="gap-2 text-xs cursor-pointer">
          <GitFork className="h-3.5 w-3.5" />
          Fork 会话
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          id="last-item"
          className={
            "gap-2 text-xs cursor-pointer text-red-600 " +
            (hover ? "bg-red-50" : "focus:text-red-600 focus:bg-red-50")
          }
        >
          <Bin className="h-3.5 w-3.5" />
          删除会话
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function Page() {
  return (
    <div className="relative" style={{ width: 700, height: 320, background: "#ffffff" }}>
      {/* sidebar panel + its 6px scrollbar, sitting behind the popup */}
      <div className="absolute left-0 top-0 h-full" style={{ width: 250, background: "#f8f8f8" }} />
      <div className="absolute" style={{ left: 37, top: 0, width: 6, height: "100%", background: "#d4d4d4" }} />

      <div className="relative h-5 w-5 mt-20 ml-5">
        <SessionMenu />
      </div>

      <div className="absolute left-300 top-110" style={{ left: 300, top: 110 }}>
        <SettingSelect
          value="codex"
          onChange={() => {}}
          placeholder="选择主题"
          prefix={
            <span className="bg-[#e0edff] text-[#2563eb] text-[10.5px] font-bold px-1.5 py-0.5 rounded leading-none">
              Aa
            </span>
          }
          options={[
            { value: "codex", label: "Codex" },
            { value: "light", label: "浅色" },
            { value: "dark", label: "深色" },
          ]}
        />
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Page />);
