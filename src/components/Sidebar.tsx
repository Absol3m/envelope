import * as Dropdown from "@radix-ui/react-dropdown-menu";
import { useEffect, useState, type DragEvent } from "react";
import { Bot, Check, ChevronsUpDown, Clock, Download, Lock, ShieldCheck, TriangleAlert, FolderPlus, HardDrive, Inbox, Layers, MoreHorizontal, PenSquare, RefreshCw, Settings } from "lucide-react";
import type { DiskEncryption, FolderRow } from "@/lib/api";
import { formatNumber, relativeTime } from "@/lib/format";
import { useShallow } from "zustand/react/shallow";
import { useComposer } from "@/lib/composer";
import { loadComposer } from "@/lib/lazy";
import { useMail } from "@/lib/store";
import { DRAG_TYPE, LOCAL_ACCOUNT, parseDragIds, isDestination, NAV_MAILBOXES, NAV_SMART, sameView, type View, type NavItem as NavItemData } from "@/lib/views";
import { cn } from "@/lib/utils";
import { dragRegion, isMac, isWindows, settingsLabel } from "@/lib/platform";
import { installUpdate, useUpdate } from "@/lib/update";
import { btnPrimary, IconButton, Spinner, Tip } from "./ui/primitives";
import { t } from "@/lib/i18n";

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
  /** Export : mbox (un fichier) ou EML (un fichier par message, dans un sous-dossier du dossier choisi). */
  const exportTo = async (f: FolderRow, format: "mbox" | "eml") => {
    try {
      const token = await backend.exportPick(format, f.name); // sélecteur natif ouvert par l'application
      if (!token) return;
      toast({ message: t("sidebar:exporting", { name: f.name }), duration: 3000 });
      const r = await backend.exportFolder(f.id, format, token);
      toast({ message: r.missing > 0 ? t("sidebar:exportedMissing", { count: r.exported, missing: r.missing }) : t("sidebar:exported", { count: r.exported }), tone: r.missing > 0 ? "error" : "default", duration: 5000 });
    } catch (e) {
      toast({ message: t("sidebar:exportFailed", { error: String(e) }), tone: "error", duration: 7000 });
    }
  };
  const commit = () => {
    const id = editing;
    const value = name.trim();
    setEditing(null);
    if (id === null || !value) return;
    void run(() => (id === "new" ? backend.createLocalFolder(value) : backend.renameLocalFolder(id, value)), id === "new" ? t("sidebar:createFailed") : t("sidebar:renameFailed"));
  };
  return (
    <>
      <div className="flex items-center px-2.5 pb-1 pt-4 max-[1100px]:hidden">
        <p className="flex-1 text-[10.5px] font-semibold uppercase tracking-wider text-faint" title={t("sidebar:localHint")}>{t("sidebar:local")}</p>
        <IconButton label={t("sidebar:import")} tooltipSide="right" onClick={() => useMail.getState().setImportOpen(true)}><Download size={14} /></IconButton>
        <IconButton label={t("sidebar:newLocalFolder")} tooltipSide="right" onClick={() => { setName(""); setEditing("new"); }}><FolderPlus size={14} /></IconButton>
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
                <button aria-label={t("sidebar:folderOptions", { name: f.name })} className="press absolute right-1 top-1/2 hidden -translate-y-1/2 rounded p-1 text-muted hover:bg-hover hover:text-fg group-hover/local:block data-[state=open]:block max-[1100px]:hidden">
                  <MoreHorizontal size={14} />
                </button>
              </Dropdown.Trigger>
              <Dropdown.Portal>
                <Dropdown.Content align="start" sideOffset={4} className="z-50 min-w-[160px] rounded-xl border border-border bg-elevated p-1 text-[13px] shadow-pop">
                  <Dropdown.Item onSelect={() => { setName(f.name); setEditing(f.id); }} className="cursor-default rounded-lg px-2.5 py-1.5 outline-none data-[highlighted]:bg-hover">{t("sidebar:rename")}</Dropdown.Item>
                  <Dropdown.Item onSelect={() => void exportTo(f, "mbox")} className="cursor-default rounded-lg px-2.5 py-1.5 outline-none data-[highlighted]:bg-hover">{t("sidebar:exportMbox")}</Dropdown.Item>
                  <Dropdown.Item onSelect={() => void exportTo(f, "eml")} className="cursor-default rounded-lg px-2.5 py-1.5 outline-none data-[highlighted]:bg-hover">{t("sidebar:exportEml")}</Dropdown.Item>
                  <Dropdown.Separator className="my-1 h-px bg-border" />
                  <Dropdown.Item onSelect={() => void run(() => backend.deleteLocalFolder(f.id), t("sidebar:deleteFailed"))} className="cursor-default rounded-lg px-2.5 py-1.5 text-danger outline-none data-[highlighted]:bg-hover">Supprimer (si vide)</Dropdown.Item>
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
      aria-label={t("sidebar:folderName")}
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

/** Comment activer le chiffrement du disque, selon le système. */
const enableEncryption = () => t(isMac ? "sidebar:encrypt.mac" : isWindows ? "sidebar:encrypt.windows" : "sidebar:encrypt.linux");

/** Pastille de chiffrement du disque : verte s'il est chiffré, triangle jaune sinon ; rien si le système ne le dit pas. */
function DiskEncryptionBadge() {
  const backend = useMail((s) => s.backend);
  const [state, setState] = useState<DiskEncryption>("unknown");
  useEffect(() => { backend.diskEncryption().then(setState, () => {}); }, [backend]);
  if (state === "unknown") return null;
  const on = state === "on";
  return (
    <Tip label={on ? t("sidebar:diskEncrypted") : `${t("sidebar:diskNotEncrypted")} ${enableEncryption()}`} side="top">
      <span role="img" aria-label={on ? t("sidebar:diskEncryptedShort") : t("sidebar:diskNotEncryptedShort")} tabIndex={0} className={cn("inline-flex h-7 w-7 items-center justify-center rounded-md", on ? "text-success" : "text-warn")}>
        {on ? <ShieldCheck size={14} /> : <TriangleAlert size={14} />}
      </span>
    </Tip>
  );
}

export function Sidebar() {
  const { accounts, folders, accountFilter, view, setView, setAccount, openSettings, backend, moveTo, scheduledCount, setScheduledOpen, lockEnabled, lockNow } = useMail(useShallow((s) => ({ lockEnabled: (s.lock?.method ?? "off") !== "off", lockNow: s.lockNow, scheduledCount: s.scheduled.length, setScheduledOpen: s.setScheduledOpen, moveTo: s.moveTo, accounts: s.accounts, folders: s.folders, accountFilter: s.accountFilter, view: s.view, setView: s.setView, setAccount: s.setAccount, openSettings: s.openSettings, backend: s.backend })));
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
                <span className="block truncate text-[13px] font-semibold leading-tight">{current ? (current.displayName ?? current.email) : t("sidebar:allMailboxes")}</span>
                <span className="block truncate text-[11px] leading-tight text-muted">{current ? current.email : t("sidebar:accountCount", { count: accounts.length })}</span>
              </span>
              <ChevronsUpDown size={14} className="shrink-0 text-faint max-[1100px]:hidden" />
            </button>
          </Dropdown.Trigger>
          <Dropdown.Portal>
            <Dropdown.Content align="start" sideOffset={6} className="anim-fade-in z-[70] w-[236px] rounded-xl bg-elevated p-1.5 shadow-pop">
              {[{ id: null as string | null, label: t("sidebar:allMailboxes"), sub: t("sidebar:unified") }, ...accounts.map((a) => ({ id: a.id as string | null, label: a.displayName ?? a.email, sub: a.email }))].map((o) => (
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
        <Tip label={t("sidebar:newMessage")} kbd="C" side="right">
          <button onClick={() => useComposer.getState().compose()} onPointerEnter={() => void loadComposer()} onFocus={() => void loadComposer()} className="press flex h-9 w-full items-center justify-center gap-2 rounded-lg bg-accent text-[13px] font-medium text-white shadow-card hover:opacity-90">
            <PenSquare size={15} /> <span className="max-[1100px]:hidden">{t("sidebar:newMessage")}</span>
          </button>
        </Tip>
      </div>

      <nav className="min-h-0 flex-1 space-y-px overflow-y-auto px-2.5 pb-3" aria-label={t("sidebar:folders")}>
        {NAV_MAILBOXES.map((i) => (
          <NavItem key={i.label} icon={i.icon} label={i.label} count={countFor(i)} active={sameView(view, i.view)} onClick={() => setView(i.view)} onDrop={dropFor(i.view)} />
        ))}

        <p className="px-2.5 pb-1 pt-4 text-[10.5px] font-semibold uppercase tracking-wider text-faint max-[1100px]:hidden">{t("sidebar:smartViews")}</p>
        <div className="hidden pt-3 max-[1100px]:block" />
        {NAV_SMART.map((i) => (
          <NavItem key={i.label} icon={i.icon} label={i.label} count={countFor(i)} active={sameView(view, i.view)} onClick={() => setView(i.view)} />
        ))}

        {scheduledCount > 0 && <NavItem icon={Clock} label={t("sidebar:scheduled")} count={scheduledCount} active={false} onClick={() => setScheduledOpen(true)} />}

        {custom.length > 0 && (
          <>
            <p className="px-2.5 pb-1 pt-4 text-[10.5px] font-semibold uppercase tracking-wider text-faint max-[1100px]:hidden">{t("sidebar:foldersTitle")}</p>
            {custom.map((f) => (
              <NavItem key={f.id} icon={Inbox} label={f.name} count={f.unreadCount} active={view.kind === "folder" && view.folderId === f.id} onClick={() => setView({ kind: "folder", folderId: f.id, name: f.name })} />
            ))}
          </>
        )}

        <LocalFolders folders={localFolders} view={view} setView={setView} moveTo={moveTo} />
      </nav>

      <UpdateBanner />

      <div className="flex items-center gap-1 border-t border-border px-2.5 py-2 max-[1100px]:flex-col">
        <div className="min-w-0 flex-1 px-1 text-[11px] text-muted max-[1100px]:hidden">
          {backend.demo ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-soft px-2 py-0.5 font-medium text-accent">{t("sidebar:demo")}</span>
          ) : errored ? (
            <span className="flex items-center gap-1.5 text-danger" title={errored.syncError ?? undefined}>● {t("sidebar:syncError")}</span>
          ) : backfillLeft > 0 ? (
            <span className="flex items-center gap-1.5" title={t("sidebar:backfillHint")}><Spinner size={11} /> {t("sidebar:backfill", { n: formatNumber(backfillLeft) })}</span>
          ) : syncing ? (
            <span className="flex items-center gap-1.5"><Spinner size={11} /> {t("sidebar:syncing")}</span>
          ) : (
            <span className="flex items-center gap-1.5"><span className="h-1.5 w-1.5 rounded-full bg-success" /> {t("sidebar:synced", { when: relativeTime(lastSync) })}</span>
          )}
        </div>
        <DiskEncryptionBadge />
        <IconButton label={t("sidebar:sync")} tooltipSide="top" onClick={() => accounts.forEach((a) => void backend.syncNow(a.id))}>
          <RefreshCw size={15} />
        </IconButton>
        {lockEnabled && (
          <IconButton label={t("sidebar:lock")} tooltipSide="top" onClick={() => void lockNow()}>
            <Lock size={15} />
          </IconButton>
        )}
        <IconButton label={t("sidebar:agent")} tooltipSide="top" onClick={() => openSettings("agent")}>
          <Bot size={15} />
        </IconButton>
        <IconButton label={settingsLabel()} kbd="mod+," tooltipSide="top" onClick={() => openSettings("accounts")}>
          <Settings size={15} />
        </IconButton>
      </div>
    </aside>
  );
}

/** Nouvelle version publiée (vérification : lib/update.ts) : installation en un clic, puis redémarrage. */
function UpdateBanner() {
  const { available, status, error } = useUpdate(useShallow((s) => ({ available: s.available, status: s.status, error: s.error })));
  if (!available) return null;
  const installing = status === "installing";
  const install = () => void installUpdate();
  return (
    <>
      <div className="mx-2.5 mb-2 rounded-lg bg-accent-soft px-3 py-2 text-[12px] max-[1100px]:hidden">
        <p className="font-medium text-accent">{t("sidebar:updateAvailable", { version: available })}</p>
        {error && <p className="mt-0.5 text-danger">{error}</p>}
        <button disabled={installing} onClick={install} className={cn(btnPrimary, "mt-1.5 h-7 gap-1.5 rounded-md px-2.5 text-[12px] shadow-none")}>
          {installing ? <><Spinner size={11} /> {t("sidebar:installing")}</> : <><Download size={13} /> {t("sidebar:install")}</>}
        </button>
      </div>
      <div className="mb-1 hidden justify-center max-[1100px]:flex">
        <IconButton label={error ?? t("sidebar:installVersion", { version: available })} tooltipSide="right" disabled={installing} onClick={install} className="text-accent">
          {installing ? <Spinner size={13} /> : <Download size={15} />}
        </IconButton>
      </div>
    </>
  );
}
