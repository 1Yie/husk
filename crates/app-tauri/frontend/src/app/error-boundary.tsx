import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * Last-resort boundary for the whole shell.
 *
 * React 18 unmounts the root when a render/lifecycle error escapes every
 * boundary, which leaves a blank white window with no way back — the same
 * symptom as a dead WebKit renderer. A small reload surface is strictly better
 * than a silent blank page, and the component stack lands in the console/log.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Husk UI crashed:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div
          role="alert"
          className="flex h-screen w-screen flex-col items-center justify-center gap-3 bg-neutral-50 p-6 text-neutral-800 dark:bg-neutral-950 dark:text-neutral-100"
        >
          <h1 className="text-[16px] font-medium">界面渲染出错</h1>
          <pre className="max-h-[40vh] max-w-[80vw] overflow-auto rounded-lg border border-neutral-200 bg-white p-3 text-[12px] whitespace-pre-wrap text-rose-600 dark:border-neutral-800 dark:bg-neutral-900 dark:text-rose-400">
            {String(this.state.error.message || this.state.error)}
          </pre>
          <button
            type="button"
            className="rounded-md border border-neutral-300 px-3 py-1.5 text-[13px] transition-colors hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-900"
            onClick={() => window.location.reload()}
          >
            重新加载
          </button>
          <p className="max-w-[560px] text-center text-[12px] text-neutral-500">
            如果反复出现，请退出并重启 Husk。诊断日志在
            <code className="mx-1 rounded bg-neutral-100 px-1 py-px dark:bg-neutral-900">
              ~/.local/share/husk/logs/husk.log
            </code>
            。
          </p>
        </div>
      );
    }
    return this.props.children;
  }
}
