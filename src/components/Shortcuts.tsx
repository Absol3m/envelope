import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { useMail } from "@/lib/store";
import { DESTINATION_IDS, DESTINATIONS, NAV_MAILBOXES, NAV_SMART } from "@/lib/views";
import { keys, settingsLabel } from "@/lib/platform";
import { IconButton, Kbd } from "./ui/primitives";
import { t as tr } from "@/lib/i18n";

const shortcuts = (): { group: string; items: [string[], string][] }[] => [
  { group: tr("shortcuts:navigation"), items: [[["J"], tr("shortcuts:next")], [["⇧", tr("shortcuts:click")], tr("shortcuts:range")], [[...keys("mod"), tr("shortcuts:click")], tr("shortcuts:addToSelection")], [keys("mod+a"), tr("reader:selectAll")], [["K"], tr("shortcuts:previous")], [["/"], tr("shortcuts:search")], [keys("mod+k"), tr("reader:palette")], [["1", "–", "4"], tr("shortcuts:tabs")], [[tr("common:keys.escape")], tr("shortcuts:closeOrDeselect")]] },
  { group: tr("shortcuts:actions"), items: [...DESTINATION_IDS.map((d): [string[], string] => [[DESTINATIONS[d].key], DESTINATIONS[d].label]), [["⌫"], tr("shortcuts:deleteForever")], [["S"], tr("shortcuts:star")], [["U"], tr("shortcuts:readUnread")], [["Z"], tr("shortcuts:undoSend")]] },
  { group: tr("shortcuts:goTo"), items: [...NAV_MAILBOXES, ...NAV_SMART].filter((i) => i.go).map((i): [string[], string] => [["G", (i.go as string).toUpperCase()], i.label]) },
  { group: tr("shortcuts:application"), items: [[keys("mod+,"), settingsLabel()], [["?"], tr("shortcuts:thisHelp")]] },
];

export function ShortcutList() {
  return (
    <div className="grid gap-x-10 gap-y-6 sm:grid-cols-2">
      {shortcuts().map((g) => (
        <section key={g.group}>
          <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">{g.group}</h3>
          <ul className="space-y-1.5">
            {g.items.map(([keys, label]) => (
              <li key={label} className="flex items-center justify-between gap-4 text-[13px]">
                <span>{label}</span>
                <span className="flex shrink-0 gap-1">{keys.map((k, i) => <Kbd key={i}>{k}</Kbd>)}</span>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

export function ShortcutsHelp() {
  const { helpOpen, setHelp } = useMail(useShallow((s) => ({ helpOpen: s.helpOpen, setHelp: s.setHelp })));
  return (
    <Dialog.Root open={helpOpen} onOpenChange={setHelp}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/35 backdrop-blur-[3px] anim-fade-in" />
        <Dialog.Content aria-describedby={undefined} className="fixed left-1/2 top-1/2 z-[91] w-[min(640px,92vw)] rounded-2xl bg-elevated p-6 shadow-pop" style={{ animation: "dialog-in 0.2s cubic-bezier(0.16,1,0.3,1) both" }}>
          <div className="mb-5 flex items-center">
            <Dialog.Title className="flex-1 text-[16px] font-semibold tracking-tight">{tr("shortcuts:title")}</Dialog.Title>
            <Dialog.Close asChild><IconButton label={tr("common:close")}><X size={16} /></IconButton></Dialog.Close>
          </div>
          <ShortcutList />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
