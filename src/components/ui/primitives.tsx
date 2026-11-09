import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { displayName, hue, initials } from "@/lib/format";

export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd className={cn("inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[5px] border border-border-strong bg-bg px-1 font-sans text-[10px] font-medium leading-none text-muted", className)}>
      {children}
    </kbd>
  );
}

export const TooltipProvider = TooltipPrimitive.Provider;

/** Infobulle avec raccourci clavier optionnel : l'interface s'explique elle-même. */
export function Tip({ label, kbd, side = "bottom", children }: { label: string; kbd?: string; side?: "top" | "bottom" | "left" | "right"; children: ReactNode }) {
  return (
    <TooltipPrimitive.Root delayDuration={350}>
      <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Content
          side={side}
          sideOffset={7}
          className="anim-fade-in z-[70] flex items-center gap-2 rounded-md bg-fg px-2 py-1 text-[11px] font-medium text-bg shadow-pop"
        >
          {label}
          {kbd && <span className="rounded bg-bg/20 px-1 text-[10px] opacity-80">{kbd}</span>}
        </TooltipPrimitive.Content>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  kbd?: string;
  active?: boolean;
  tooltipSide?: "top" | "bottom" | "left" | "right";
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, kbd, active, tooltipSide, className, children, ...rest },
  ref,
) {
  return (
    <Tip label={label} kbd={kbd} side={tooltipSide}>
      <button
        ref={ref}
        aria-label={label}
        className={cn(
          "press inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted hover:bg-hover hover:text-fg disabled:pointer-events-none disabled:opacity-40",
          active && "text-accent",
          className,
        )}
        {...rest}
      >
        {children}
      </button>
    </Tip>
  );
});

/** Avatar à dégradé stable par expéditeur : repère visuel rapide, sans photo à charger. */
export function Avatar({ name, address, size = 32 }: { name?: string | null; address?: string | null; size?: number }) {
  const label = displayName(name, address);
  const h = hue((address ?? label).toLowerCase());
  return (
    <span
      aria-hidden
      className="inline-flex shrink-0 select-none items-center justify-center rounded-full font-semibold text-white"
      style={{
        width: size,
        height: size,
        fontSize: size * 0.38,
        background: `linear-gradient(135deg, hsl(${h} 62% 58%), hsl(${(h + 38) % 360} 66% 46%))`,
        boxShadow: "inset 0 0 0 1px rgb(255 255 255 / 0.18)",
      }}
    >
      {initials(label)}
    </span>
  );
}

export function Spinner({ size = 14 }: { size?: number }) {
  return (
    <span
      role="progressbar"
      aria-label="Chargement"
      className="inline-block rounded-full border-2 border-current border-t-transparent opacity-70"
      style={{ width: size, height: size, animation: "spin 0.7s linear infinite" }}
    />
  );
}

/** Styles partagés des champs et boutons de formulaire (surchargeables avec `cn`). */
export const inputCls =
  "selectable h-9 w-full rounded-lg border border-border bg-bg px-3 text-[13px] outline-none transition-colors placeholder:text-faint focus:border-accent/60 focus:ring-[3px] focus:ring-accent-soft";
const btnCls = "press inline-flex h-9 items-center justify-center gap-2 rounded-lg px-3.5 text-[13px] font-medium disabled:opacity-50";
export const btnPrimary = `${btnCls} bg-accent text-accent-fg hover:bg-accent-hover shadow-card`;
export const btnGhost = `${btnCls} border border-border bg-bg hover:bg-hover`;
