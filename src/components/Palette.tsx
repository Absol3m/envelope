import { Command } from "cmdk";
import { Bot, Forward, Inbox, Keyboard, Mail, MailOpen, Monitor, Moon, PenSquare, RefreshCw, Reply, ReplyAll, Search, Settings, Star, Sun, Trash2, Undo2 } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useState } from "react";
import { useComposer } from "@/lib/composer";
import { actionView, selectionIds, useMail } from "@/lib/store";
import { DESTINATIONS, destinationsFor, isPurgeView, NAV_MAILBOXES, NAV_SMART } from "@/lib/views";
import { Kbd } from "./ui/primitives";
import { keys, SETTINGS_LABEL } from "@/lib/platform";

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
    <Command.Dialog open={s.paletteOpen} onOpenChange={(o) => (o ? s.setPalette(true) : close())} label="Palette de commandes" loop>
      <Command.Input value={text} onValueChange={setText} placeholder="Rechercher une action, un dossier, un e-mail…" />
      <Command.List>
        <Command.Empty>Aucune commande trouvée.</Command.Empty>

        {text.trim() && (
          <Command.Group heading="Recherche">
            <Item icon={Search} value={`rechercher ${text}`} onSelect={run(() => s.setQuery(text.trim()))}>Rechercher « {text.trim()} » dans les e-mails</Item>
          </Command.Group>
        )}

        {picked.length > 0 && (
          <Command.Group heading={many ? `${picked.length} conversations` : "Conversation"}>
            {destinationsFor(actionView(s, ids)).map((d) => (
              <Item key={d} icon={DESTINATIONS[d].icon} hint={[DESTINATIONS[d].key]} onSelect={run(() => s.moveTo(ids, d))}>{DESTINATIONS[d].label}</Item>
            ))}
            {isPurgeView(actionView(s, ids)) && <Item icon={Trash2} hint={["⌫"]} onSelect={run(() => void s.deleteForever(ids))}>Supprimer définitivement</Item>}
            <Item icon={Star} hint={["S"]} onSelect={run(() => s.toggleStar(ids))}>{anyUnstarred ? "Suivre" : "Ne plus suivre"}</Item>
            <Item icon={anyUnread ? MailOpen : Mail} hint={["U"]} onSelect={run(() => s.toggleRead(ids))}>{anyUnread ? "Marquer comme lu" : "Marquer comme non lu"}</Item>
          </Command.Group>
        )}

        <Command.Group heading="Aller à">
          {[...NAV_MAILBOXES, ...NAV_SMART].map((i) => (
            <Item key={i.label} icon={i.icon} hint={i.go ? ["G", i.go.toUpperCase()] : undefined} onSelect={run(() => s.setView(i.view))}>{i.label}</Item>
          ))}
        </Command.Group>

        <Command.Group heading="Affichage">
          <Item icon={Sun} onSelect={run(() => s.setTheme("light"))}>Thème clair</Item>
          <Item icon={Moon} onSelect={run(() => s.setTheme("dark"))}>Thème sombre</Item>
          <Item icon={Monitor} onSelect={run(() => s.setTheme("system"))}>Thème du système</Item>
          <Item icon={Inbox} onSelect={run(() => s.setDensity(s.density === "compact" ? "comfortable" : "compact"))}>Densité : {s.density === "compact" ? "confortable" : "compacte"}</Item>
        </Command.Group>

        <Command.Group heading="Écrire">
          <Item icon={PenSquare} hint={["C"]} onSelect={run(() => useComposer.getState().compose())}>Nouveau message</Item>
          <Item icon={Reply} hint={["R"]} onSelect={run(() => useComposer.getState().respond("reply"))}>Répondre</Item>
          <Item icon={ReplyAll} hint={["A"]} onSelect={run(() => useComposer.getState().respond("replyAll"))}>Répondre à tous</Item>
          <Item icon={Forward} hint={["F"]} onSelect={run(() => useComposer.getState().respond("forward"))}>Transférer</Item>
          {saved && <Item icon={PenSquare} onSelect={run(() => useComposer.getState().resume())}>Reprendre le brouillon</Item>}
        </Command.Group>

        <Command.Group heading="Application">
          <Item icon={Undo2} hint={["Z"]} onSelect={run(() => void useComposer.getState().undoSend())}>Annuler l'envoi</Item>
          <Item icon={RefreshCw} onSelect={run(() => s.accounts.forEach((a) => void s.backend.syncNow(a.id)))}>Synchroniser maintenant</Item>
          <Item icon={Bot} onSelect={run(() => s.openSettings("agent"))}>Agent IA (serveur MCP)</Item>
          <Item icon={Settings} hint={keys("mod+,")} onSelect={run(() => s.openSettings("accounts"))}>{SETTINGS_LABEL}</Item>
          <Item icon={Keyboard} hint={["?"]} onSelect={run(() => s.setHelp(true))}>Raccourcis clavier</Item>
        </Command.Group>
      </Command.List>
    </Command.Dialog>
  );
}
