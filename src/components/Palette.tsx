import { Command } from "cmdk";
import { Archive, Bot, Inbox, Keyboard, Mail, MailOpen, Monitor, Moon, RefreshCw, Search, Settings, Star, Sun, Undo2 } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useState } from "react";
import { useMail } from "@/lib/store";
import { NAV_MAILBOXES, NAV_SMART } from "@/lib/views";
import { Kbd } from "./ui/primitives";

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

/** ⌘K : toute action de l'application, et la recherche, au même endroit. */
export function Palette() {
  const s = useMail();
  const [text, setText] = useState("");
  const close = () => { s.setPalette(false); setText(""); };
  const run = (fn: () => void) => () => { close(); fn(); };
  const id = s.selectedId;
  const t = s.threads.find((x) => x.id === id);

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

        {t && (
          <Command.Group heading="Conversation">
            <Item icon={Archive} hint={["E"]} onSelect={run(() => s.archive(t.id))}>Archiver</Item>
            <Item icon={Star} hint={["S"]} onSelect={run(() => s.toggleStar(t.id))}>{t.isStarred ? "Ne plus suivre" : "Suivre"}</Item>
            <Item icon={t.unreadCount > 0 ? MailOpen : Mail} hint={["U"]} onSelect={run(() => s.toggleRead(t.id))}>{t.unreadCount > 0 ? "Marquer comme lu" : "Marquer comme non lu"}</Item>
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

        <Command.Group heading="Application">
          <Item icon={Undo2} hint={["Z"]} onSelect={run(() => s.undo())}>Annuler la dernière archive</Item>
          <Item icon={RefreshCw} onSelect={run(() => s.accounts.forEach((a) => void s.backend.syncNow(a.id)))}>Synchroniser maintenant</Item>
          <Item icon={Bot} onSelect={run(() => s.openSettings("agent"))}>Agent IA (serveur MCP)</Item>
          <Item icon={Settings} hint={["⌘", ","]} onSelect={run(() => s.openSettings("accounts"))}>Réglages</Item>
          <Item icon={Keyboard} hint={["?"]} onSelect={run(() => s.setHelp(true))}>Raccourcis clavier</Item>
        </Command.Group>
      </Command.List>
    </Command.Dialog>
  );
}
