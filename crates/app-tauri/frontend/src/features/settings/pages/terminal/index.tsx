// 设置 › 终端 — colour scheme, font, cursor and scrollback for the built-in
// terminal. Every change applies live to open terminals: the panel subscribes
// to the same store.

import { useMemo } from "react";
import {
  CursorText,
  Layers,
  PaintRoller,
  PanelBottomCloseDashed,
  ScanLine,
  TextQuote,
} from "@keyline-icons/react";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Slider } from "@/components/ui/slider";
import {
  SettingRow,
  SettingSelect,
  SettingsCards,
  SettingsSection,
} from "@/features/settings/components";
import {
  TERMINAL_FONT_SIZE,
  TERMINAL_LINE_HEIGHTS,
  TERMINAL_SCHEMES,
  TERMINAL_SCROLLBACK,
  isAppDark,
  resolveTerminalFont,
  resolveTerminalPalette,
  useTerminalPrefs,
  type TerminalCursorStyle,
  type TerminalPalette,
  type TerminalSchemeId,
} from "@/features/terminal/preferences";
import { terminalFontOptions, useSystemTerminalFonts } from "@/features/terminal/fonts";

const CURSORS: { value: TerminalCursorStyle; label: string }[] = [
  { value: "block", label: "方块" },
  { value: "bar", label: "竖线" },
  { value: "underline", label: "下划线" },
];

/** A few lines of fake shell output in the scheme's own colours. */
function SchemePreview({
  palette,
  font,
}: {
  palette: TerminalPalette;
  font?: string;
}) {
  return (
    <div
      aria-hidden="true"
      className="w-full overflow-hidden rounded-lg border border-hairline px-2.5 py-2 font-mono text-[10.5px] leading-[1.55]"
      style={{
        background: palette.background,
        color: palette.foreground,
        fontFamily: font,
      }}
    >
      <div className="truncate">
        <span style={{ color: palette.green }}>husk</span>
        <span> on </span>
        <span style={{ color: palette.magenta }}>main</span>
        <span style={{ color: palette.red }}> [!?]</span>
      </div>
      <div className="truncate">
        <span style={{ color: palette.cyan }}>❯</span> cargo test
      </div>
      <div className="truncate">
        <span style={{ color: palette.green }}>ok</span>
        <span style={{ color: palette.yellow }}> warn</span>
        <span style={{ color: palette.blue }}> info</span>
        <span style={{ color: palette.brightBlack }}> 12ms</span>
      </div>
    </div>
  );
}

function SchemeSwatches({ palette }: { palette: TerminalPalette }) {
  const colours = [
    palette.red,
    palette.green,
    palette.yellow,
    palette.blue,
    palette.magenta,
    palette.cyan,
  ];
  return (
    <span aria-hidden="true" className="flex flex-none items-center gap-0.5">
      {colours.map((c, i) => (
        <span
          key={i}
          className="h-2.5 w-2.5 rounded-full"
          style={{ background: c }}
        />
      ))}
    </span>
  );
}

export function TerminalSettings() {
  const prefs = useTerminalPrefs();
  const { fonts, status: fontStatus } = useSystemTerminalFonts();
  const fontOptions = useMemo(() => terminalFontOptions(fonts, prefs.fontFamily), [fonts, prefs.fontFamily]);
  const { update, reset } = prefs;
  const dark = isAppDark();
  const css = getComputedStyle(document.documentElement);
  const token = (name: string) =>
    css.getPropertyValue(name).trim() || undefined;
  const previewFont = resolveTerminalFont(prefs.fontFamily);

  return (
    <div className="flex w-full min-w-0 flex-col gap-8">
      <SettingsSection
        title="配色"
        actions={
          <Button size="sm" variant="outline" onClick={reset}>
            恢复默认
          </Button>
        }
      >
        {/* Keep the longest scheme name, six swatches and selection marker on one row. */}
        <SettingsCards
          value={prefs.scheme}
          onChange={(v) => update({ scheme: v as TerminalSchemeId })}
          className="grid-cols-[repeat(auto-fill,minmax(min(100%,16rem),1fr))]"
          options={TERMINAL_SCHEMES.map((scheme) => {
            const palette = resolveTerminalPalette(scheme.id, dark, token);
            return {
              value: scheme.id,
              label: scheme.label,
              badge: <SchemeSwatches palette={palette} />,
              preview: <SchemePreview palette={palette} font={previewFont} />,
            };
          })}
        />
      </SettingsSection>

      <SettingsSection title="字体">
        <KvList>
          <KvListContent>
            <SettingRow
              title="字体"
              description={fontStatus === "error" ? "系统字体读取失败，仍可使用内置 JetBrains Mono"
                : fontStatus === "loading" ? "正在读取系统字体，默认使用内置 JetBrains Mono"
                : "系统等宽字体优先；不可用时回退到内置 JetBrains Mono"}
              icon={<TextQuote className="h-4 w-4" />}
              wideControl
            >
              <SettingSelect
                value={prefs.fontFamily}
                onChange={(v) => update({ fontFamily: v })}
                ariaLabel="终端字体"
                options={fontOptions}
              />
            </SettingRow>
            <SettingRow
              title="字号"
              description={`${prefs.fontSize} px`}
              icon={<CursorText className="h-4 w-4" />}
              wideControl
            >
              <Slider
                aria-label="终端字号"
                min={TERMINAL_FONT_SIZE.min}
                max={TERMINAL_FONT_SIZE.max}
                step={1}
                value={[prefs.fontSize]}
                onValueChange={([v]) => update({ fontSize: v })}
              />
            </SettingRow>
            <SettingRow
              title="行高"
              icon={<ScanLine className="h-4 w-4" />}
              wideControl
            >
              <SettingSelect
                value={String(prefs.lineHeight)}
                onChange={(v) => update({ lineHeight: Number(v) })}
                ariaLabel="终端行高"
                options={TERMINAL_LINE_HEIGHTS.map((v) => ({
                  value: String(v),
                  label: v.toFixed(1),
                }))}
              />
            </SettingRow>
          </KvListContent>
        </KvList>
      </SettingsSection>

      <SettingsSection title="行为">
        <KvList>
          <KvListContent>
            <SettingRow
              title="光标样式"
              icon={<PaintRoller className="h-4 w-4" />}
              wideControl
            >
              <SettingSelect
                value={prefs.cursorStyle}
                onChange={(v) =>
                  update({ cursorStyle: v as TerminalCursorStyle })
                }
                ariaLabel="光标样式"
                options={CURSORS}
              />
            </SettingRow>
            <SettingRow
              title="光标闪烁"
              icon={<CursorText className="h-4 w-4" />}
            >
              <Switch
                aria-label="光标闪烁"
                checked={prefs.cursorBlink}
                onCheckedChange={(v) => update({ cursorBlink: v })}
              />
            </SettingRow>
            <SettingRow
              title="回滚行数"
              description="每个终端保留的历史输出行数"
              icon={<Layers className="h-4 w-4" />}
              wideControl
            >
              <SettingSelect
                value={String(prefs.scrollback)}
                onChange={(v) => update({ scrollback: Number(v) })}
                ariaLabel="回滚行数"
                options={TERMINAL_SCROLLBACK.map((v) => ({
                  value: String(v),
                  label: `${v.toLocaleString()} 行`,
                }))}
              />
            </SettingRow>
            <SettingRow
              title="正常退出时关闭标签"
              description="exit 或 Ctrl-D 结束 shell 后自动关闭终端面板"
              icon={<PanelBottomCloseDashed className="h-4 w-4" />}
            >
              <Switch
                aria-label="正常退出时关闭标签"
                checked={prefs.closeOnExit}
                onCheckedChange={(v) => update({ closeOnExit: v })}
              />
            </SettingRow>
          </KvListContent>
        </KvList>
      </SettingsSection>
    </div>
  );
}
