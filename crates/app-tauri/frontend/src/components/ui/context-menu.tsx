import * as React from "react";
import * as ContextMenuPrimitive from "@radix-ui/react-context-menu";

import { cn } from "@/lib/utils";
import { usePopperPlacedRef } from "./use-popper-placed";

// Same visual contract as dropdown-menu.tsx — one menu look across the app.
// modal={false}: Radix's modal branch mounts a focus trap + body scroll-lock
// whose extra work delayed first paint — the top-left flash on right-click.
const ContextMenu = (
  props: React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Root>
) => <ContextMenuPrimitive.Root modal={false} {...props} />;
ContextMenu.displayName = "ContextMenu";

// `data-allow-contextmenu` marks the trigger for the app-wide native-menu
// guard (App.tsx): the global contextmenu suppressor must not preventDefault
// on our own triggers or the custom menu never opens.
const ContextMenuTrigger = React.forwardRef<
  React.ElementRef<typeof ContextMenuPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Trigger>
>(({ ...props }, ref) => (
  <ContextMenuPrimitive.Trigger
    ref={ref}
    data-allow-contextmenu=""
    {...props}
  />
));
ContextMenuTrigger.displayName = ContextMenuPrimitive.Trigger.displayName;
const ContextMenuGroup = ContextMenuPrimitive.Group;
const ContextMenuPortal = ContextMenuPrimitive.Portal;
const ContextMenuSub = ContextMenuPrimitive.Sub;
const ContextMenuRadioGroup = ContextMenuPrimitive.RadioGroup;

const ContextMenuSubTrigger = React.forwardRef<
  React.ElementRef<typeof ContextMenuPrimitive.SubTrigger>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.SubTrigger> & {
    inset?: boolean;
  }
>(({ className, inset, children, ...props }, ref) => (
  <ContextMenuPrimitive.SubTrigger
    ref={ref}
    className={cn(
      "flex cursor-default select-none items-center rounded-lg px-2 py-1.5 text-sm outline-none focus:bg-neutral-100 data-[state=open]:bg-neutral-100",
      inset && "pl-8",
      className
    )}
    {...props}
  >
    {children}
  </ContextMenuPrimitive.SubTrigger>
));
ContextMenuSubTrigger.displayName =
  ContextMenuPrimitive.SubTrigger.displayName;

const ContextMenuSubContent = React.forwardRef<
  React.ElementRef<typeof ContextMenuPrimitive.SubContent>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.SubContent>
>(({ className, ...props }, ref) => {
  const { placed, setRef } = usePopperPlacedRef(ref);
  return (
  <ContextMenuPrimitive.SubContent
    ref={setRef}
    className={cn(
      "z-50 min-w-[8rem] overflow-hidden rounded-xl border border-neutral-200 bg-white p-1 text-neutral-900 shadow-popup duration-100 ease-out data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
      // Radix parks unpositioned content above the viewport; an engine that
      // paints it first puts the menu in the corner. Never paint it.
      !placed && "invisible",
      className
    )}
    {...props}
  />
  );
});
ContextMenuSubContent.displayName =
  ContextMenuPrimitive.SubContent.displayName;

const ContextMenuContent = React.forwardRef<
  React.ElementRef<typeof ContextMenuPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Content>
>(({ className, onCloseAutoFocus, ...props }, ref) => {
  const { placed, setRef } = usePopperPlacedRef(ref);
  return (
  <ContextMenuPrimitive.Portal>
    <ContextMenuPrimitive.Content
      ref={setRef}
      // Radix hard-codes the *submenu* placement here (`side="right"`,
      // `align="start"`, `sideOffset=2`) AFTER the caller's props, and omits
      // those props from the public type — so placement is not ours to choose,
      // and a click near a window edge gets clamped to the edge with 0px
      // padding. `collisionPadding` is not overridden, so at least every
      // context menu now keeps a gap from the window edges. (The composer,
      // which sits at the very bottom of the window and must flip *up*, uses an
      // anchored DropdownMenu instead — see ComposerMenu.)
      collisionPadding={8}
      onCloseAutoFocus={(e) => {
        e.preventDefault();
        onCloseAutoFocus?.(e);
      }}
      className={cn(
        "z-50 min-w-[8rem] overflow-hidden rounded-xl border border-neutral-200 bg-white p-1 text-neutral-900 shadow-popup duration-100 ease-out data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
        !placed && "invisible",
        className
      )}
      {...props}
    />
  </ContextMenuPrimitive.Portal>
  );
});
ContextMenuContent.displayName = ContextMenuPrimitive.Content.displayName;

const ContextMenuItem = React.forwardRef<
  React.ElementRef<typeof ContextMenuPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Item> & {
    inset?: boolean;
  }
>(({ className, inset, ...props }, ref) => (
  <ContextMenuPrimitive.Item
    ref={ref}
    className={cn(
      "relative flex cursor-default select-none items-center rounded-lg px-2 py-1.5 text-sm outline-none transition-colors focus:bg-neutral-100 focus:text-neutral-900 data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
      inset && "pl-8",
      className
    )}
    {...props}
  />
));
ContextMenuItem.displayName = ContextMenuPrimitive.Item.displayName;

const ContextMenuSeparator = React.forwardRef<
  React.ElementRef<typeof ContextMenuPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Separator>
>(({ className, ...props }, ref) => (
  <ContextMenuPrimitive.Separator
    ref={ref}
    className={cn("-mx-1 my-1 h-px bg-neutral-100", className)}
    {...props}
  />
));
ContextMenuSeparator.displayName = ContextMenuPrimitive.Separator.displayName;

const ContextMenuLabel = React.forwardRef<
  React.ElementRef<typeof ContextMenuPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Label> & {
    inset?: boolean;
  }
>(({ className, inset, ...props }, ref) => (
  <ContextMenuPrimitive.Label
    ref={ref}
    className={cn(
      "px-2 py-1.5 text-xs text-neutral-500 font-normal",
      inset && "pl-8",
      className
    )}
    {...props}
  />
));
ContextMenuLabel.displayName = ContextMenuPrimitive.Label.displayName;

const ContextMenuShortcut = ({
  className,
  ...props
}: React.HTMLAttributes<HTMLSpanElement>) => {
  return (
    <span
      className={cn("ml-auto text-xs tracking-widest opacity-60", className)}
      {...props}
    />
  );
};
ContextMenuShortcut.displayName = "ContextMenuShortcut";

export {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuGroup,
  ContextMenuPortal,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuRadioGroup,
};
