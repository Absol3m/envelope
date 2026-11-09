import * as Dropdown from "@radix-ui/react-dropdown-menu";
import { Bot, Check, ChevronsUpDown, Inbox, Layers, RefreshCw, Settings } from "lucide-react";
import { relativeTime } from "@/lib/format";
import { useShallow } from "zustand/react/shallow";
import { useMail } from "@/lib/store";
import { NAV_MAILBOXES, NAV_SMART, sameView, type NavItem as NavItemData } from "@/lib/views";
import { cn } from "@/lib/utils";
import { IconButton, Spinner } from "./ui/primitives";

type Icon = NavItemData["icon"];

function NavItem({ icon: Icon, label, count, active, onClick }: { icon: Icon; label: string; count?: number; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      title={label}
      aria-current={active ? "page" : undefined}
      className={cn(
        "press group flex h-[30px] w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] font-medium max-[1100px]:justify-center max-[1100px]:px-0",
        active ? "bg-active text-fg" : "text-muted hover:bg-hover hover:text-fg",
      )}
    >
      <Icon size={16} strokeWidth={active ? 2.1 : 1.8} className={cn("shrink-0", active && "text-accent")} />
      <span className="flex-1 truncate max-[1100px]:hidden">{label}</span>
      {count ? <span className={cn("tnum text-[11px] font-semibold max-[1100px]:hidden", active ? "text-fg" : "text-faint")}>{count}</span> : null}
    </button>
  );
}

export function Sidebar() {
  const { accounts, folders, accountFilter, view, setView, setAccount, openSettings, backend } = useMail(useShallow((s) => ({ accounts: s.accounts, folders: s.folders, accountFilter: s.accountFilter, view: s.view, setView: s.setView, setAccount: s.setAccount, openSettings: s.openSettings, backend: s.backend })));
  const scoped = folders.filter((f) => !accountFilter || f.accountId === accountFilter);
  const unread = (role: string) => scoped.filter((f) => f.role === role).reduce((n, f) => n + f.unreadCount, 0);
  /** Seuls la boîte de réception et « Non lus » affichent un compteur. */
  const countFor = (i: NavItemData) => (i.view.kind === "unread" || (i.view.kind === "role" && i.view.role === "inbox") ? unread("inbox") : undefined);
  const custom = scoped.filter((f) => f.role === null).slice(0, 8);
  const current = accounts.find((a) => a.id === accountFilter);
  const syncing = accounts.some((a) => a.syncStatus === "syncing");
  const errored = accounts.find((a) => a.syncStatus === "error");
  const lastSync = accounts.map((a) => a.lastSyncAt).filter(Boolean).sort().pop() ?? null;

  return (
    <aside className="flex h-full min-h-0 flex-col border-r border-border bg-sidebar">
      {/* Zone de glisser (feux tricolores macOS en surimpression) */}
      <div data-tauri-drag-region className="h-[46px] shrink-0" />

      <div className="px-2.5 pb-2">
        <Dropdown.Root>
          <Dropdown.Trigger asChild>
            <button className="press flex h-10 w-full items-center gap-2.5 rounded-lg px-2 text-left hover:bg-hover max-[1100px]:justify-center max-[1100px]:px-0">
              <span className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-accent to-[#8b5cf6] text-white shadow-card">
                {current ? <span className="text-[11px] font-bold">{(current.displayName ?? current.email)[0].toUpperCase()}</span> : <Layers size={14} />}
              </span>
              <span className="min-w-0 flex-1 max-[1100px]:hidden">
                <span className="block truncate text-[13px] font-semibold leading-tight">{current ? (current.displayName ?? current.email) : "Toutes les boîtes"}</span>
                <span className="block truncate text-[11px] leading-tight text-muted">{current ? current.email : `${accounts.length} compte${accounts.length > 1 ? "s" : ""}`}</span>
              </span>
              <ChevronsUpDown size={14} className="shrink-0 text-faint max-[1100px]:hidden" />
            </button>
          </Dropdown.Trigger>
          <Dropdown.Portal>
            <Dropdown.Content align="start" sideOffset={6} className="anim-fade-in z-[70] w-[236px] rounded-xl bg-elevated p-1.5 shadow-pop">
              {[{ id: null as string | null, label: "Toutes les boîtes", sub: "Boîte unifiée" }, ...accounts.map((a) => ({ id: a.id as string | null, label: a.displayName ?? a.email, sub: a.email }))].map((o) => (
                <Dropdown.Item
                  key={o.id ?? "all"}
                  onSelect={() => setAccount(o.id)}
                  className="flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-[13px] outline-none data-[highlighted]:bg-hover"
                >
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{o.label}</span>
                    <span className="block truncate text-[11px] text-muted">{o.sub}</span>
                  </span>
                  {accountFilter === o.id && <Check size={14} className="text-accent" />}
                </Dropdown.Item>
              ))}
              <Dropdown.Separator className="my-1 h-px bg-border" />
              <Dropdown.Item onSelect={() => openSettings("accounts")} className="flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-[13px] text-muted outline-none data-[highlighted]:bg-hover data-[highlighted]:text-fg">
                <Settings size={14} /> Gérer les comptes…
              </Dropdown.Item>
            </Dropdown.Content>
          </Dropdown.Portal>
        </Dropdown.Root>
      </div>

      <nav className="min-h-0 flex-1 space-y-px overflow-y-auto px-2.5 pb-3" aria-label="Dossiers">
        {NAV_MAILBOXES.map((i) => (
          <NavItem key={i.label} icon={i.icon} label={i.label} count={countFor(i)} active={sameView(view, i.view)} onClick={() => setView(i.view)} />
        ))}

        <p className="px-2.5 pb-1 pt-4 text-[10.5px] font-semibold uppercase tracking-wider text-faint max-[1100px]:hidden">Vues intelligentes</p>
        <div className="hidden pt-3 max-[1100px]:block" />
        {NAV_SMART.map((i) => (
          <NavItem key={i.label} icon={i.icon} label={i.label} count={countFor(i)} active={sameView(view, i.view)} onClick={() => setView(i.view)} />
        ))}

        {custom.length > 0 && (
          <>
            <p className="px-2.5 pb-1 pt-4 text-[10.5px] font-semibold uppercase tracking-wider text-faint max-[1100px]:hidden">Dossiers</p>
            {custom.map((f) => (
              <NavItem key={f.id} icon={Inbox} label={f.name} count={f.unreadCount} active={view.kind === "folder" && view.folderId === f.id} onClick={() => setView({ kind: "folder", folderId: f.id, name: f.name })} />
            ))}
          </>
        )}
      </nav>

      <div className="flex items-center gap-1 border-t border-border px-2.5 py-2 max-[1100px]:flex-col">
        <div className="min-w-0 flex-1 px-1 text-[11px] text-muted max-[1100px]:hidden">
          {backend.demo ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-soft px-2 py-0.5 font-medium text-accent">Mode démo</span>
          ) : errored ? (
            <span className="flex items-center gap-1.5 text-danger" title={errored.syncError ?? undefined}>● Erreur de synchro</span>
          ) : syncing ? (
            <span className="flex items-center gap-1.5"><Spinner size={11} /> Synchronisation…</span>
          ) : (
            <span className="flex items-center gap-1.5"><span className="h-1.5 w-1.5 rounded-full bg-success" /> Sync {relativeTime(lastSync)}</span>
          )}
        </div>
        <IconButton label="Synchroniser" tooltipSide="top" onClick={() => accounts.forEach((a) => void backend.syncNow(a.id))}>
          <RefreshCw size={15} />
        </IconButton>
        <IconButton label="Agent IA (MCP)" tooltipSide="top" onClick={() => openSettings("agent")}>
          <Bot size={15} />
        </IconButton>
        <IconButton label="Réglages" kbd="⌘," tooltipSide="top" onClick={() => openSettings("accounts")}>
          <Settings size={15} />
        </IconButton>
      </div>
    </aside>
  );
}
