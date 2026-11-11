import * as Dropdown from "@radix-ui/react-dropdown-menu";
import { useState, type DragEvent } from "react";
import { Bot, Check, ChevronsUpDown, Download, FolderPlus, HardDrive, Inbox, Layers, MoreHorizontal, PenSquare, RefreshCw, Settings } from "lucide-react";
import type { FolderRow } from "@/lib/api";
import { relativeTime } from "@/lib/format";
import { useShallow } from "zustand/react/shallow";
import { useComposer } from "@/lib/composer";
import { useMail } from "@/lib/store";
import { DRAG_TYPE, LOCAL_ACCOUNT, parseDragIds, isDestination, NAV_MAILBOXES, NAV_SMART, sameView, type View, type NavItem as NavItemData } from "@/lib/views";
import { cn } from "@/lib/utils";
import { dragRegion, isMac, SETTINGS_LABEL } from "@/lib/platform";
import { IconButton, Spinner, Tip } from "./ui/primitives";

type Icon = NavItemData["icon"];

/** Dépôt d'une conversation glissée depuis la liste : `onDrop` reçoit son identifiant. */
function NavItem({ icon: Icon, label, count, active, onClick, onDrop }: { icon: Icon; label: string; count?: number; active: boolean; onClick: () => void; onDrop?: (threadIds: number[]) => void }) {
  const [over, setOver] = useState(false);
  const accepts = (e: DragEvent) => onDrop !== undefined && e.dataTransfer.types.includes(DRAG_TYPE);
  return (
    <button
      onClick={onClick}
      data-drop={onDrop ? "" : undefined}
      data-over={over ? "" : undefined}
      onDragOver={(e) => { if (accepts(e)) { e.preventDefault(); e.dataTransfer.dropEffect = "move"; setOver(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node)) setOver(false); }}
      onDrop={(e) => {
        setOver(false);
        const ids = parseDragIds(e.dataTransfer.getData(DRAG_TYPE));
        if (accepts(e) && ids.length > 0) { e.preventDefault(); onDrop?.(ids); }
      }}
      title={label}
      aria-current={active ? "page" : undefined}
      className={cn(
        "nav-target press group flex h-[30px] w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] font-medium max-[1100px]:justify-center max-[1100px]:px-0",
        active ? "bg-active text-fg" : "text-muted hover:bg-hover hover:text-fg",
      )}
    >
      <Icon size={16} strokeWidth={active ? 2.1 : 1.8} className={cn("shrink-0", active && "text-accent")} />
      <span className="flex-1 truncate max-[1100px]:hidden">{label}</span>
      {count ? <span className={cn("tnum text-[11px] font-semibold max-[1100px]:hidden", active ? "text-fg" : "text-faint")}>{count}</span> : null}
    </button>
  );
}

/** Dossiers locaux (« Sur cet ordinateur ») : créer, renommer, supprimer (si vide) ; on y dépose des conversations. */
function LocalFolders({ folders, view, setView, moveTo }: { folders: FolderRow[]; view: View; setView(v: View): void; moveTo(ids: number[], to: "local", folderId: number): void }) {
  const { backend, reload, toast } = useMail(useShallow((s) => ({ backend: s.backend, reload: s.reload, toast: s.toast })));
  // `editing` : dossier en cours de renommage (identifiant) ou « new » ; null sinon.
  const [editing, setEditing] = useState<number | "new" | null>(null);
  const [name, setName] = useState("");
  const run = async (action: () => Promise<unknown>, failure: string) => {
    try {
      await action();
      await reload({ silent: true });
    } catch (e) {
      toast({ message: `${failure} — ${String(e)}`, tone: "error", duration: 6000 });
    }
  };
  const commit = () => {
    const id = editing;
    const value = name.trim();
    setEditing(null);
    if (id === null || !value) return;
    void run(() => (id === "new" ? backend.createLocalFolder(value) : backend.renameLocalFolder(id, value)), id === "new" ? "Création impossible" : "Renommage impossible");
  };
  return (
    <>
      <div className="flex items-center px-2.5 pb-1 pt-4 max-[1100px]:hidden">
        <p className="flex-1 text-[10.5px] font-semibold uppercase tracking-wider text-faint" title="Ces dossiers n'existent que sur cet ordinateur">Sur cet ordinateur</p>
        <IconButton label="Importer des dossiers locaux (Thunderbird…)" tooltipSide="right" onClick={() => useMail.getState().setImportOpen(true)}><Download size={14} /></IconButton>
        <IconButton label="Nouveau dossier local" tooltipSide="right" onClick={() => { setName(""); setEditing("new"); }}><FolderPlus size={14} /></IconButton>
      </div>
      <div className="hidden pt-3 max-[1100px]:block" />
      {folders.map((f) =>
        editing === f.id ? (
          <NameInput key={f.id} value={name} onChange={setName} onDone={commit} onCancel={() => setEditing(null)} />
        ) : (
          <div key={f.id} className="group/local relative">
            <NavItem
              icon={HardDrive}
              label={f.name}
              count={f.unreadCount}
              active={view.kind === "folder" && view.folderId === f.id}
              onClick={() => setView({ kind: "folder", folderId: f.id, name: f.name, local: true })}
              onDrop={(ids) => moveTo(ids, "local", f.id)}
            />
            <Dropdown.Root>
              <Dropdown.Trigger asChild>
                <button aria-label={`Options du dossier ${f.name}`} className="press absolute right-1 top-1/2 hidden -translate-y-1/2 rounded p-1 text-muted hover:bg-hover hover:text-fg group-hover/local:block data-[state=open]:block max-[1100px]:hidden">
                  <MoreHorizontal size={14} />
                </button>
              </Dropdown.Trigger>
              <Dropdown.Portal>
                <Dropdown.Content align="start" sideOffset={4} className="z-50 min-w-[160px] rounded-xl border border-border bg-elevated p-1 text-[13px] shadow-pop">
                  <Dropdown.Item onSelect={() => { setName(f.name); setEditing(f.id); }} className="cursor-default rounded-lg px-2.5 py-1.5 outline-none data-[highlighted]:bg-hover">Renommer…</Dropdown.Item>
                  <Dropdown.Item onSelect={() => void run(() => backend.deleteLocalFolder(f.id), "Suppression impossible")} className="cursor-default rounded-lg px-2.5 py-1.5 text-danger outline-none data-[highlighted]:bg-hover">Supprimer (si vide)</Dropdown.Item>
                </Dropdown.Content>
              </Dropdown.Portal>
            </Dropdown.Root>
          </div>
        ),
      )}
      {editing === "new" && <NameInput value={name} onChange={setName} onDone={commit} onCancel={() => setEditing(null)} />}
    </>
  );
}

function NameInput({ value, onChange, onDone, onCancel }: { value: string; onChange(v: string): void; onDone(): void; onCancel(): void }) {
  return (
    <input
      autoFocus
      value={value}
      maxLength={80}
      aria-label="Nom du dossier"
      onChange={(e) => onChange(e.target.value)}
      onBlur={onDone}
      onKeyDown={(e) => {
        if (e.key === "Enter") onDone();
        else if (e.key === "Escape") { e.stopPropagation(); onCancel(); }
      }}
      className="selectable my-px h-[30px] w-full rounded-md border border-accent/60 bg-bg px-2.5 text-[13px] outline-none ring-[3px] ring-accent-soft"
    />
  );
}

export function Sidebar() {
  const { accounts, folders, accountFilter, view, setView, setAccount, openSettings, backend, moveTo } = useMail(useShallow((s) => ({ moveTo: s.moveTo, accounts: s.accounts, folders: s.folders, accountFilter: s.accountFilter, view: s.view, setView: s.setView, setAccount: s.setAccount, openSettings: s.openSettings, backend: s.backend })));
  const scoped = folders.filter((f) => f.accountId !== LOCAL_ACCOUNT && (!accountFilter || f.accountId === accountFilter));
  const localFolders = folders.filter((f) => f.accountId === LOCAL_ACCOUNT);
  const unread = (role: string) => scoped.filter((f) => f.role === role).reduce((n, f) => n + f.unreadCount, 0);
  /** Boîtes où l'on peut déposer une conversation (archives, corbeille, spam, réception), sauf la vue affichée. */
  const dropFor = (v: View) => {
    const role = v.kind === "role" && isDestination(v.role) ? v.role : undefined;
    return role && !sameView(v, view) ? (ids: number[]) => moveTo(ids, role) : undefined;
  };
  /** Seuls la boîte de réception et « Non lus » affichent un compteur. */
  const countFor = (i: NavItemData) => (i.view.kind === "unread" || (i.view.kind === "role" && i.view.role === "inbox") ? unread("inbox") : undefined);
  const custom = scoped.filter((f) => f.role === null).slice(0, 8);
  const current = accounts.find((a) => a.id === accountFilter);
  const syncing = accounts.some((a) => a.syncStatus === "syncing");
  const backfillLeft = useMail((s) => Object.values(s.backfill).reduce((n, v) => n + v, 0));
  const errored = accounts.find((a) => a.syncStatus === "error");
  const lastSync = accounts.map((a) => a.lastSyncAt).filter(Boolean).sort().pop() ?? null;

  return (
    <aside className="row-span-2 flex h-full min-h-0 flex-col border-r border-border bg-sidebar">
      {/* macOS : zone de glisser sous les feux tricolores en surimpression ; ailleurs, barre de titre native */}
      <div {...dragRegion} className={cn("shrink-0", isMac ? "h-[46px]" : "h-3")} />

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

      <div className="px-2.5 pb-2">
        <Tip label="Nouveau message" kbd="C" side="right">
          <button onClick={() => useComposer.getState().compose()} className="press flex h-9 w-full items-center justify-center gap-2 rounded-lg bg-accent text-[13px] font-medium text-white shadow-card hover:opacity-90">
            <PenSquare size={15} /> <span className="max-[1100px]:hidden">Nouveau message</span>
          </button>
        </Tip>
      </div>

      <nav className="min-h-0 flex-1 space-y-px overflow-y-auto px-2.5 pb-3" aria-label="Dossiers">
        {NAV_MAILBOXES.map((i) => (
          <NavItem key={i.label} icon={i.icon} label={i.label} count={countFor(i)} active={sameView(view, i.view)} onClick={() => setView(i.view)} onDrop={dropFor(i.view)} />
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

        <LocalFolders folders={localFolders} view={view} setView={setView} moveTo={moveTo} />
      </nav>

      <div className="flex items-center gap-1 border-t border-border px-2.5 py-2 max-[1100px]:flex-col">
        <div className="min-w-0 flex-1 px-1 text-[11px] text-muted max-[1100px]:hidden">
          {backend.demo ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-soft px-2 py-0.5 font-medium text-accent">Mode démo</span>
          ) : errored ? (
            <span className="flex items-center gap-1.5 text-danger" title={errored.syncError ?? undefined}>● Erreur de synchro</span>
          ) : backfillLeft > 0 ? (
            <span className="flex items-center gap-1.5" title="Les anciens messages sont téléchargés en arrière-plan : ils deviennent cherchables au fur et à mesure."><Spinner size={11} /> Archive : {backfillLeft.toLocaleString("fr-FR")} restants</span>
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
        <IconButton label={SETTINGS_LABEL} kbd="mod+," tooltipSide="top" onClick={() => openSettings("accounts")}>
          <Settings size={15} />
        </IconButton>
      </div>
    </aside>
  );
}
