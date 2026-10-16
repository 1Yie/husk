import { useEffect, useRef, useState } from "react";
import { Eye, EyeOff, RefreshCw } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { getProviderApiKey } from "@/lib/agent-ipc/sessions";
import { cn } from "@/lib/utils";
import { toast } from "sonner";

// Matches the backend's keep-stored-value sentinel. Never render it as a key.
const KEY_MASK = "\u2022\u2022\u2022\u2022";

export function ApiKeyInput({ value, onChange, provider }: {
  value: string;
  onChange: (value: string) => void;
  provider?: string;
}) {
  const [visible, setVisible] = useState(false);
  const [revealed, setRevealed] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const request = useRef(0);
  const stored = value === KEY_MASK;

  useEffect(() => {
    setVisible(false);
    setRevealed(null);
    setLoading(false);
    return () => { request.current += 1; };
  }, [provider]);

  const toggle = async () => {
    if (visible) {
      setVisible(false);
      setRevealed(null);
      return;
    }
    if (!stored) {
      setVisible(true);
      return;
    }
    if (!provider || loading) return;
    const currentRequest = ++request.current;
    setLoading(true);
    try {
      const key = await getProviderApiKey(provider);
      if (request.current !== currentRequest) return;
      // Revealing must not turn an unchanged key into an edited draft.
      setRevealed(key);
      setVisible(true);
    } catch {
      if (request.current === currentRequest) toast.error("读取 API Key 失败");
    } finally {
      if (request.current === currentRequest) setLoading(false);
    }
  };

  return (
    <div className="relative min-w-0">
      <Input
        type={visible ? "text" : "password"}
        value={stored ? (visible ? revealed ?? "" : "") : value}
        onChange={(e) => {
          // Ignore a reveal response that arrives after a replacement was typed.
          request.current += 1;
          setLoading(false);
          setRevealed(null);
          onChange(e.target.value);
        }}
        aria-label="API Key"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        placeholder={loading ? "读取中…" : stored ? "已保存，输入新 Key 可替换" : "输入 API Key 或 env:VAR"}
        className={cn("h-8 pr-10 text-xs", visible ? "font-mono" : "font-sans")}
      />
      <Button
        type="button"
        size="icon"
        variant="ghost"
        className="absolute right-1 top-1/2 h-6 w-6 -translate-y-1/2 text-neutral-500"
        aria-label={visible ? "隐藏 API Key" : "显示 API Key"}
        title={visible ? "隐藏 API Key" : "显示 API Key"}
        aria-pressed={visible}
        aria-busy={loading}
        disabled={loading || (stored && !provider)}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => void toggle()}
      >
        {loading ? <RefreshCw className="h-3.5 w-3.5 animate-spin" />
          : visible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
      </Button>
    </div>
  );
}
