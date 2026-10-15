import { render, type RenderOptions } from "@testing-library/react";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { ReactElement, ReactNode } from "react";

export function TestProviders({ children }: { children: ReactNode }) {
  return <TooltipProvider delayDuration={0}>{children}</TooltipProvider>;
}
export function renderWithProviders(ui: ReactElement, options?: Omit<RenderOptions, "wrapper">) {
  return render(ui, { wrapper: TestProviders, ...options });
}
