import { Check, Inbox, Mail, MailOpen, Paperclip, SearchX, Star, Trash2 } from "lucide-react";
import { startThreadDrag } from "@/lib/dragImage";
import type { MouseEvent } from "react";
import { defaultRangeExtractor, useVirtualizer, type Range } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import type { ThreadSummary } from "@/lib/api";
import { categorize, CATEGORY_LABELS, type Category } from "@/lib/categories";
import { displayName, groupLabel, listDate } from "@/lib/format";
import { dragIds, useMail, visibleThreads } from "@/lib/store";
import { DESTINATIONS, destinationsFor, isEmptiableView, isInboxView, isPurgeView, placeOf, viewTitle, type Destination, type View } from "@/lib/views";
import { cn } from "@/lib/utils";
import { dragRegion } from "@/lib/platform";
import { Avatar, IconButton } from "./ui/primitives";

interface RowProps {
  t: ThreadSummary;
  selected: boolean;
  compact: boolean;
  previews: boolean;
  checked: boolean;
  onSelect(id: number, e: MouseEvent): void;
  destinations: Destination[];
  purge: boolean;
  /** Résultat de recherche : nom du dossier d'où vient la conversation. */
  folderName?: string;
  onDelete(id: number): void;
  onMove(id: number, to: Destination): void;
  onStar(id: number): void;
  onToggleRead(id: number): void;
  /** Rang (à partir de 1) et nombre total : la liste n'a dans le DOM que les lignes visibles. */
  pos: number;
  total: number;
}

const Row = memo(function Row({ t, selected, compact, previews, checked, onSelect, destinations, purge, folderName, onDelete, onMove, onStar, onToggleRead, pos, total }: RowProps) {
  const unread = t.unreadCount > 0;
  const active = selected || checked;
  const sender = displayName(t.fromName, t.fromAddr);
  // Conversation à plusieurs : prénoms des participants plutôt que le seul dernier expéditeur.
  const who = t.participants && t.messageCount > 1 ? t.participants.split(", ").map((p) => p.split(" ")[0]).join(", ") : sender;
  return (
    <div
      role="option"
      aria-selected={selected}
      aria-posinset={pos}
      aria-setsize={total}
      data-thread={t.id}
      data-checked={checked ? "" : undefined}
      draggable
      onDragStart={(e) => startThreadDrag(e, t, dragIds(useMail.getState(), t.id))}
      onClick={(e) => onSelect(t.id, e)}
      className={cn(
        "group relative mx-1.5 flex cursor-default select-none gap-3 rounded-lg px-3 transition-colors",
        compact ? "py-[7px]" : "py-[11px]",
        active ? "bg-accent-soft" : "hover:bg-hover",
      )}
    >
      {active && <span className="absolute inset-y-2 left-0 w-[3px] rounded-full bg-accent" />}
      <div className="flex w-2 shrink-0 items-start pt-[7px]">
        {unread && <span aria-label="Non lu" className="h-2 w-2 rounded-full bg-accent shadow-[0_0_0_3px_var(--accent-soft)]" />}
      </div>
      {!compact && (
        <div className="pt-px">
          <Avatar name={t.fromName} address={t.fromAddr} size={34} />
        </div>
      )}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className={cn("truncate text-[13px]", unread ? "font-semibold text-fg" : "font-medium text-fg/85")}>{who}</span>
          {t.messageCount > 1 && <span className="tnum shrink-0 rounded bg-hover px-1 text-[10.5px] font-medium text-muted">{t.messageCount}</span>}
          <span className="ml-auto flex shrink-0 items-center gap-1.5 text-[11.5px] text-muted">
            {folderName && <span className="max-w-[90px] truncate rounded bg-hover px-1.5 py-px text-[10.5px] font-medium text-muted" title={`Dossier : ${folderName}`}>{folderName}</span>}
            {t.hasAttachments && <Paperclip size={12} className="text-faint" />}
            {t.isStarred && <Star size={12} className="fill-star text-star" />}
            <span className={cn("tnum", unread && "font-medium text-fg/80")}>{listDate(t.lastMessageAt)}</span>
          </span>
        </div>
        <div className={cn("truncate text-[13px]", unread ? "font-medium text-fg" : "text-fg/80")}>{t.subject || "(sans objet)"}</div>
        {previews && <div className="truncate text-[12.5px] text-muted">{t.snippet || "—"}</div>}
      </div>

      {/* Actions rapides au survol : mêmes gestes que le clavier (E, #, !, S, U). */}
      <div className={cn("absolute right-2 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded-lg bg-elevated p-0.5 shadow-card group-hover:flex", selected && "bg-bg")}>
        {destinations.map((d) => {
          const { label, icon: Icon, key } = DESTINATIONS[d];
          return (
            <IconButton key={d} label={label} kbd={key} onClick={(e) => { e.stopPropagation(); onMove(t.id, d); }}>
              <Icon size={15} />
            </IconButton>
          );
        })}
        {purge && (
          <IconButton label="Supprimer définitivement" kbd="⌫" onClick={(e) => { e.stopPropagation(); onDelete(t.id); }}>
            <Trash2 size={15} className="text-danger" />
          </IconButton>
        )}
        <IconButton label={t.isStarred ? "Ne plus suivre" : "Suivre"} kbd="S" onClick={(e) => { e.stopPropagation(); onStar(t.id); }}>
          <Star size={15} className={t.isStarred ? "fill-star text-star" : ""} />
        </IconButton>
        <IconButton label={unread ? "Marquer comme lu" : "Marquer comme non lu"} kbd="U" onClick={(e) => { e.stopPropagation(); onToggleRead(t.id); }}>
          {unread ? <MailOpen size={15} /> : <Mail size={15} />}
        </IconButton>
      </div>
    </div>
  );
});

function Skeletons({ compact }: { compact: boolean }) {
  return (
    <div className="space-y-1 px-3 pt-2" aria-hidden>
      {Array.from({ length: 9 }, (_, i) => (
        <div key={i} className="flex gap-3 px-3 py-[11px]" style={{ opacity: 1 - i * 0.09 }}>
          <div className="w-2" />
          {!compact && <div className="skeleton h-[34px] w-[34px] rounded-full" />}
          <div className="flex-1 space-y-2">
            <div className="skeleton h-3 w-1/3" />
            <div className="skeleton h-3 w-4/5" />
            <div className="skeleton h-3 w-3/5" />
          </div>
        </div>
      ))}
    </div>
  );
}

function Empty({ search, view, category }: { search: string; view: View; category: Category }) {
  const inboxZero = isInboxView(view) && !search && category === "all";
  const Icon = search ? SearchX : inboxZero ? Check : Inbox;
  const title = search ? `Aucun résultat pour « ${search} »` : inboxZero ? "Boîte de réception à zéro" : "Rien ici pour l'instant";
  const hint = search
    ? "Essayez d'autres mots-clés : la recherche ignore les accents et accepte les débuts de mots. La recherche couvre tous les dossiers sauf corbeille et spam (dans:tout les inclut). Opérateurs : de:  à:  objet:  dans:  avant:  apres:  pj  non-lu  suivi"
    : inboxZero
      ? "Tout est traité. Profitez-en."
      : "Les conversations de cette vue apparaîtront ici.";
  return (
    <div className="anim-fade-up flex h-full flex-col items-center justify-center px-10 text-center">
      <span className={cn("mb-4 inline-flex h-14 w-14 items-center justify-center rounded-2xl", inboxZero ? "bg-success/10 text-success" : "bg-hover text-faint")}>
        <Icon size={26} strokeWidth={1.6} />
      </span>
      <p className="text-[14px] font-semibold">{title}</p>
      <p className="mt-1 max-w-[280px] text-[12.5px] text-muted">{hint}</p>
    </div>
  );
}

export function ThreadList() {
  const { folders, view, category, query, selectedId, density, previews, threadsLoading, hasMore, threads, hiddenIds, checked, toggleCheck, rangeCheck, setCategory, loadMore, select, moveTo, deleteForever, emptyView, toggleStar, toggleRead } = useMail(
    useShallow((s) => ({ folders: s.folders, view: s.view, category: s.category, query: s.query, selectedId: s.selectedId, density: s.density, previews: s.previews, threadsLoading: s.threadsLoading, hasMore: s.hasMore, threads: s.threads, hiddenIds: s.hiddenIds, checked: s.checked, toggleCheck: s.toggleCheck, rangeCheck: s.rangeCheck, setCategory: s.setCategory, loadMore: s.loadMore, select: s.select, moveTo: s.moveTo, deleteForever: s.deleteForever, emptyView: s.emptyView, toggleStar: s.toggleStar, toggleRead: s.toggleRead })),
  );
  const scroller = useRef<HTMLDivElement>(null);
  /** Clic : ouvre ; Maj+clic : plage ; Cmd/Ctrl+clic : ajoute ou retire. */
  const onRowClick = useCallback((id: number, e: MouseEvent) => {
    if (e.shiftKey) rangeCheck(id);
    else if (e.metaKey || e.ctrlKey) toggleCheck(id);
    else select(id);
  }, [select, toggleCheck, rangeCheck]);
  const searching = query.trim() !== "";
  const purge = isPurgeView(view);
  const destinations = useMemo(() => destinationsFor(view), [view]);
  const list = useMemo(() => visibleThreads({ threads, hiddenIds, category, view, query }), [threads, hiddenIds, category, view, query]);
  const isInbox = isInboxView(view) && !query.trim();

  const counts = useMemo(() => {
    const c: Record<Category, number> = { all: 0, priority: 0, newsletters: 0, notifications: 0 };
    for (const t of threads) {
      if (hiddenIds.has(t.id) || t.unreadCount === 0) continue;
      c.all++;
      c[categorize(t)]++;
    }
    return c;
  }, [threads, hiddenIds]);

  // Groupes par date (en-têtes collants).
  const rows = useMemo(() => {
    const out: ({ kind: "h"; label: string; key: string } | { kind: "t"; t: ThreadSummary; pos: number })[] = [];
    let last = "";
    list.forEach((t, i) => {
      const g = groupLabel(t.lastMessageAt);
      if (g !== last) {
        out.push({ kind: "h", label: g, key: `h-${g}-${t.id}` }); // unique même si une date revenait (liste non triée)
        last = g;
      }
      out.push({ kind: "t", t, pos: i + 1 });
    });
    return out;
  }, [list]);

  const compact = density === "compact";
  const headerIndexes = useMemo(() => rows.flatMap((r, i) => (r.kind === "h" ? [i] : [])), [rows]);
  const activeHeader = useRef(0);
  // Fonctions de la bibliothèque : leur identité doit rester stable (celle de `getItemKey` invalide ses mesures, donc
  // les recalculerait pour toutes les lignes à chaque image de défilement).
  const getItemKey = useCallback((i: number) => { const r = rows[i]; return r ? (r.kind === "h" ? r.key : r.t.id) : i; }, [rows]);
  const rangeExtractor = useCallback((range: Range) => {
    // En-tête de la dernière date au-dessus de la fenêtre (recherche binaire dans les indices, triés) : il reste collé en haut.
    let lo = 0, hi = headerIndexes.length - 1, found = 0;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (headerIndexes[mid] <= range.startIndex) { found = headerIndexes[mid]; lo = mid + 1; } else hi = mid - 1;
    }
    activeHeader.current = found;
    return [...new Set([found, ...defaultRangeExtractor(range)])].sort((x, y) => x - y);
  }, [headerIndexes]);
  // Liste fenêtrée : seules les lignes visibles (et une marge) existent dans le DOM, quel que soit le nombre de
  // conversations chargées. Hauteurs mesurées à l'affichage ; l'en-tête de date en cours reste collé en haut.
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    // Hauteurs mesurées dans l'application (confortable + aperçu : 78 px, en-tête : 32 px) : plus l'estimation est juste,
    // plus le défilement vers une ligne lointaine tombe juste avant même qu'elle soit mesurée.
    estimateSize: (i) => (rows[i]?.kind === "h" ? 32 : compact ? (previews ? 70 : 50) : previews ? 78 : 58),
    overscan: 8,
    getItemKey,
    rangeExtractor,
  });
  const items = virtualizer.getVirtualItems();

  // Garde la conversation sélectionnée visible (navigation clavier) : par indice, la ligne peut ne pas être dans le DOM.
  useEffect(() => {
    if (selectedId === null) return;
    const i = rows.findIndex((r) => r.kind === "t" && r.t.id === selectedId);
    if (i < 0) return;
    virtualizer.scrollToIndex(i, { align: "auto" });
    // Les lignes lointaines n'étaient pas mesurées : une fois affichées, leurs vraies hauteurs décalent la cible ; on recadre
    // (mesuré : sans cela, la ligne cible n'est pas toujours dans la fenêtre à 4 000 conversations et plus).
    const again = [80, 250].map((ms) => setTimeout(() => virtualizer.scrollToIndex(i, { align: "auto" }), ms));
    return () => again.forEach(clearTimeout);
    // Seule la sélection déclenche le recadrage : dépendre de `rows` le referait à chaque page chargée.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

  // Pagination infinie : la page suivante part quand on approche de la fin de la liste chargée.
  const lastVisible = items.length ? items[items.length - 1].index : -1;
  useEffect(() => {
    if (hasMore && rows.length > 0 && lastVisible >= rows.length - 12) void loadMore();
  }, [hasMore, rows.length, lastVisible, loadMore]);

  return (
    <section className="flex h-full min-h-0 flex-col border-r border-border bg-bg-subtle" aria-label="Conversations">
      <header className="shrink-0 px-4 pt-3">
        <div {...dragRegion} className="flex h-[30px] items-baseline gap-2">
          <h1 {...dragRegion} className="pointer-events-none text-[17px] font-semibold tracking-tight">{query.trim() ? "Recherche" : viewTitle(view)}</h1>
          {list.length > 0 && <span {...dragRegion} className="tnum pointer-events-none text-[12px] text-faint">{list.length}</span>}
          {isEmptiableView(view) && list.length > 0 && !query.trim() && (
            <button onClick={() => void emptyView()} className="press ml-auto rounded-md px-2 py-0.5 text-[12px] font-medium text-danger hover:bg-hover">
              {view.role === "trash" ? "Vider la corbeille" : "Vider le spam"}
            </button>
          )}
        </div>


        {isInbox && (
          <div role="tablist" aria-label="Smart Inbox" className="-mx-1 mt-2.5 flex gap-0.5 overflow-x-auto pb-px">
            {(Object.keys(CATEGORY_LABELS) as Category[]).map((c, i) => (
              <button
                key={c}
                role="tab"
                aria-selected={category === c}
                onClick={() => setCategory(c)}
                title={`${CATEGORY_LABELS[c]} (${i + 1})`}
                className={cn("press flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2 text-[12.5px] font-medium", category === c ? "bg-active text-fg" : "text-muted hover:bg-hover hover:text-fg")}
              >
                {CATEGORY_LABELS[c]}
                {counts[c] > 0 && (c === "all" || category === c) && (
                  <span className={cn("tnum rounded-full px-1.5 text-[10.5px] font-semibold", category === c ? "bg-accent text-accent-fg" : "bg-hover text-muted")}>{counts[c]}</span>
                )}
              </button>
            ))}
          </div>
        )}
        <div className="mt-2 h-px bg-border" />
      </header>

      <div ref={scroller} role="listbox" aria-label="Liste des conversations" className="min-h-0 flex-1 overflow-y-auto pb-6 pt-1">
        {threadsLoading && list.length === 0 ? (
          <Skeletons compact={compact} />
        ) : list.length === 0 ? (
          <Empty search={query.trim()} view={view} category={category} />
        ) : (
          <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
            {items.map((v) => {
              const r = rows[v.index];
              if (!r) return null;
              const sticky = v.index === activeHeader.current && r.kind === "h";
              const style = sticky ? { position: "sticky" as const, top: 0, zIndex: 10 } : { position: "absolute" as const, top: 0, left: 0, width: "100%", transform: `translateY(${v.start}px)` };
              return (
                <div key={v.key} data-index={v.index} ref={virtualizer.measureElement} style={style}>
                  {r.kind === "h" ? (
                    <div className="bg-bg-subtle/90 px-5 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-faint backdrop-blur-md">{r.label}</div>
                  ) : (
                    <Row t={r.t} pos={r.pos} total={list.length} selected={r.t.id === selectedId} compact={compact} previews={previews} checked={checked.has(r.t.id)} onSelect={onRowClick} destinations={searching ? destinationsFor(placeOf(r.t, folders) ?? view) : destinations} purge={searching ? isPurgeView(placeOf(r.t, folders) ?? view) : purge} folderName={searching ? folders.find((f) => f.id === r.t.folderId)?.name : undefined} onDelete={deleteForever} onMove={moveTo} onStar={toggleStar} onToggleRead={toggleRead} />
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}
