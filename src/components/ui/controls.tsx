import * as RadioGroup from "@radix-ui/react-radio-group";
import * as SwitchPrimitive from "@radix-ui/react-switch";
import type { ComponentType } from "react";

// Radix plutôt que des <input> natifs : un groupe de boutons radio stylé (flèches, un seul arrêt de tabulation) et un
// interrupteur accessible que les contrôles natifs ne donnent pas sans les recoder. Dans un fichier à part : ils ne
// servent qu'au panneau de réglages (chargé à la demande) ; dans `primitives.tsx`, leurs paquets entreraient dans le
// chargement initial.

/** Choix exclusif entre quelques options (groupe de boutons radio : flèches pour passer de l'une à l'autre). */
export function Segmented<T extends string>({ value, onChange, options }: { value: T; onChange(v: T): void; options: { value: T; label: string; icon?: ComponentType<{ size?: number }> }[] }) {
  return (
    <RadioGroup.Root value={value} onValueChange={(v) => onChange(v as T)} orientation="horizontal" className="inline-flex rounded-lg bg-hover p-0.5">
      {options.map((o) => (
        <RadioGroup.Item
          key={o.value}
          value={o.value}
          className="press flex h-8 items-center gap-1.5 rounded-md px-3 text-[12.5px] font-medium text-muted hover:text-fg data-[state=checked]:bg-bg data-[state=checked]:text-fg data-[state=checked]:shadow-card"
        >
          {o.icon && <o.icon size={14} />}
          {o.label}
        </RadioGroup.Item>
      ))}
    </RadioGroup.Root>
  );
}

/** Interrupteur (role switch, espace pour basculer) ; `label` : son nom pour les lecteurs d'écran. */
export function Switch({ checked, onChange, label }: { checked: boolean; onChange(v: boolean): void; label: string }) {
  return (
    <SwitchPrimitive.Root checked={checked} onCheckedChange={onChange} aria-label={label} className="relative h-[22px] w-[38px] shrink-0 rounded-full bg-active transition-colors data-[state=checked]:bg-accent">
      <SwitchPrimitive.Thumb className="block h-[18px] w-[18px] translate-x-[2px] rounded-full bg-white shadow transition-transform data-[state=checked]:translate-x-[18px]" />
    </SwitchPrimitive.Root>
  );
}
