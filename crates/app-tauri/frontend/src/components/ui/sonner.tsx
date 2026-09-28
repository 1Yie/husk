import { Toaster as Sonner } from "sonner";
// Statically bundle sonner's stylesheet instead of relying on its runtime
// <style> injection — inside WKWebView the injected rules can fail to
// apply, which leaves the toast <ol> in the normal flow at the top of
// #root and wrecks the whole layout.
import "sonner/dist/styles.css";

type ToasterProps = React.ComponentProps<typeof Sonner>;

const Toaster = ({ ...props }: ToasterProps) => {
  return (
    <Sonner
      className="toaster group"
      toastOptions={{
        classNames: {
          toast:
            "group toast group-[.toaster]:bg-white group-[.toaster]:text-neutral-950 group-[.toaster]:border-neutral-200 group-[.toaster]:shadow-popup",
          description: "group-[.toast]:text-neutral-500",
          actionButton:
            "group-[.toast]:bg-neutral-900 group-[.toast]:text-neutral-50",
          cancelButton:
            "group-[.toast]:bg-neutral-100 group-[.toast]:text-neutral-500",
        },
      }}
      {...props}
    />
  );
};

export { Toaster };
