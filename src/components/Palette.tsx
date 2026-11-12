import { Command } from "cmdk";
import { Bot, Download, Forward, Inbox, Keyboard, Lock, Mail, MailOpen, Monitor, Moon, PenSquare, RefreshCw, Reply, ReplyAll, Search, Settings, Star, Sun, Trash2, Undo2 } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useState } from "react";
import { useComposer } from "@/lib/composer";
import { actionView, selectionIds, useMail } from "@/lib/store";
import { DESTINATIONS, destinationsFor, isPurgeView, NAV_MAILBOXES, NAV_SMART } from "@/lib/views";
import { Kbd } from "./ui/primitives";
import { keys, settingsLabel } from "@/lib/platform";
import { t as tr } from "@/lib/i18n";

type Icon = ComponentType<{ size?: number }>;

function Item({ icon: Icon, children, hint, onSelect, value }: { icon: Icon; children: ReactNode; hint?: string[]; onSelect(): void; value?: string }) {
  return (
    <Command.Item value={value} onSelect={onSelect}>
      <Icon size={16} />
      <span className="flex-1 truncate">{children}</span>
      {hint && <span className="flex gap-1">{hint.map((k) => <Kbd key={k}>{k}</Kbd>)}</span>}
    </Command.Item>
  );
}

/** ⌘K / Ctrl+K : toute action de l'application, et la recherche, au même endroit. */
export function Palette() {
  const s = useMail();
  const saved = useComposer((c) => c.saved !== null);
  const [text, setText] = useState("");
  const close = () => { s.setPalette(false); setText(""); };
  const run = (fn: () => void) => () => { close(); fn(); };
  const ids = selectionIds(s);
  const idSet = new Set(ids);
  const picked = s.threads.filter((x) => idSet.has(x.id));
  const many = picked.length > 1;
  const anyUnread = picked.some((x) => x.unreadCount > 0);
  const anyUnstarred = picked.some((x) => !x.isStarred);

  return (
    <Command.Dialog open={s.paletteOpen} onOpenChange={(o) => (o ? s.setPalette(true) : close())} label={tr("reader:palette")} loop>
      <Command.Input value={text} onValueChange={setText} placeholder={tr("palette:placeholder")} />
      <Command.List>
        <Command.Empty>{tr("palette:empty")}</Command.Empty>

        {text.trim() && (
          <Command.Group heading={tr("list:search")}>
            <Item icon={Search} value={`rechercher ${text}`} onSelect={run(() => s.setQuery(text.trim()))}>{tr("palette:searchFor", { text: text.trim() })}</Item>
          </Command.Group>
        )}

        {picked.length > 0 && (
          <Command.Group heading={many ? tr("list:selectedCount", { count: picked.length }) : tr("palette:conversation")}>
            {destinationsFor(actionView(s, ids)).map((d) => (
              <Item key={d} icon={DESTINATIONS[d].icon} hint={[DESTINATIONS[d].key]} onSelect={run(() => s.moveTo(ids, d))}>{DESTINATIONS[d].label}</Item>
            ))}
            {isPurgeView(actionView(s, ids)) && <Item icon={Trash2} hint={["⌫"]} onSelect={run(() => void s.deleteForever(ids))}>{tr("topbar:deleteForever")}</Item>}
            <Item icon={Star} hint={["S"]} onSelect={run(() => s.toggleStar(ids))}>{anyUnstarred ? tr("topbar:star") : tr("topbar:unstar")}</Item>
            <Item icon={anyUnread ? MailOpen : Mail} hint={["U"]} onSelect={run(() => s.toggleRead(ids))}>{anyUnread ? tr("topbar:markRead") : tr("topbar:markUnread")}</Item>
          </Command.Group>
        )}

        <Command.Group heading={tr("palette:goTo")}>
          {[...NAV_MAILBOXES, ...NAV_SMART].map((i) => (
            <Item key={i.label} icon={i.icon} hint={i.go ? ["G", i.go.toUpperCase()] : undefined} onSelect={run(() => s.setView(i.view))}>{i.label}</Item>
          ))}
        </Command.Group>

        <Command.Group heading={tr("palette:display")}>
          <Item icon={Sun} onSelect={run(() => s.setTheme("light"))}>{tr("palette:themeLight")}</Item>
          <Item icon={Moon} onSelect={run(() => s.setTheme("dark"))}>{tr("palette:themeDark")}</Item>
          <Item icon={Monitor} onSelect={run(() => s.setTheme("system"))}>{tr("palette:themeSystem")}</Item>
          <Item icon={Inbox} onSelect={run(() => s.setDensity(s.density === "compact" ? "comfortable" : "compact"))}>{tr("palette:density", { value: s.density === "compact" ? tr("settings:appearance.comfortable").toLowerCase() : tr("settings:appearance.compact").toLowerCase() })}</Item>
        </Command.Group>

        <Command.Group heading={tr("palette:write")}>
          <Item icon={PenSquare} hint={["C"]} onSelect={run(() => useComposer.getState().compose())}>{tr("sidebar:newMessage")}</Item>
          <Item icon={Reply} hint={["R"]} onSelect={run(() => useComposer.getState().respond("reply"))}>{tr("topbar:reply")}</Item>
          <Item icon={ReplyAll} hint={["A"]} onSelect={run(() => useComposer.getState().respond("replyAll"))}>{tr("topbar:replyAll")}</Item>
          <Item icon={Forward} hint={["F"]} onSelect={run(() => useComposer.getState().respond("forward"))}>{tr("topbar:forward")}</Item>
          {saved && <Item icon={PenSquare} onSelect={run(() => useComposer.getState().resume())}>{tr("palette:resumeDraft")}</Item>}
        </Command.Group>

        <Command.Group heading={tr("shortcuts:application")}>
          {s.lock && s.lock.method !== "off" && <Item icon={Lock} onSelect={run(() => void s.lockNow())}>{tr("sidebar:lock")}</Item>}
          <Item icon={Undo2} hint={["Z"]} onSelect={run(() => void useComposer.getState().undoSend())}>{tr("shortcuts:undoSend")}</Item>
          <Item icon={RefreshCw} onSelect={run(() => s.accounts.forEach((a) => void s.backend.syncNow(a.id)))}>{tr("palette:syncNow")}</Item>
          <Item icon={Download} onSelect={run(() => s.setImportOpen(true))}>{tr("sidebar:import")}</Item>
          <Item icon={Bot} onSelect={run(() => s.openSettings("agent"))}>{tr("palette:agent")}</Item>
          <Item icon={Settings} hint={keys("mod+,")} onSelect={run(() => s.openSettings("accounts"))}>{settingsLabel()}</Item>
          <Item icon={Keyboard} hint={["?"]} onSelect={run(() => s.setHelp(true))}>{tr("shortcuts:title")}</Item>
        </Command.Group>
      </Command.List>
    </Command.Dialog>
  );
}
