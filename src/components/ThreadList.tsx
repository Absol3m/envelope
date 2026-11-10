import { Archive, Check, Inbox, Mail, MailOpen, Paperclip, Search, SearchX, Star, X } from "lucide-react";
import { memo, useEffect, useMemo, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import type { ThreadSummary } from "@/lib/api";
import { categorize, CATEGORY_LABELS, type Category } from "@/lib/categories";
import { displayName, groupLabel, listDate } from "@/lib/format";
import { useMail, visibleThreads } from "@/lib/store";
import { isInboxView, viewTitle, type View } from "@/lib/views";
import { cn } from "@/lib/utils";
import { dragRegion } from "@/lib/platform";
import { Avatar, IconButton, Kbd } from "./ui/primitives";

interface RowProps {
  t: ThreadSummary;
  selected: boolean;
  compact: boolean;
  previews: boolean;
  onSelect(id: number): void;
  onArchive(id: number): void;
  onStar(id: number): void;
  onToggleRead(id: number): void;
}

const Row = memo(function Row({ t, selected, compact, previews, onSelect, onArchive, onStar, onToggleRead }: RowProps) {
  const unread = t.unreadCount > 0;
  const sender = displayName(t.fromName, t.fromAddr);
  // Conversation à plusieurs : prénoms des participants plutôt que le seul dernier expéditeur.
  const who = t.participants && t.messageCount > 1 ? t.participants.split(", ").map((p) => p.split(" ")[0]).join(", ") : sender;
  return (
    <div
      role="option"
      aria-selected={selected}
      data-thread={t.id}
      onClick={() => onSelect(t.id)}
      className={cn(
        "group relative mx-1.5 flex cursor-default gap-3 rounded-lg px-3 transition-colors",
        compact ? "py-[7px]" : "py-[11px]",
        selected ? "bg-accent-soft" : "hover:bg-hover",
      )}
    >
      {selected && <span className="absolute inset-y-2 left-0 w-[3px] rounded-full bg-accent" />}
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
            {t.hasAttachments && <Paperclip size={12} className="text-faint" />}
            {t.isStarred && <Star size={12} className="fill-star text-star" />}
            <span className={cn("tnum", unread && "font-medium text-fg/80")}>{listDate(t.lastMessageAt)}</span>
          </span>
        </div>
        <div className={cn("truncate text-[13px]", unread ? "font-medium text-fg" : "text-fg/80")}>{t.subject || "(sans objet)"}</div>
        {previews && <div className="truncate text-[12.5px] text-muted">{t.snippet || "—"}</div>}
      </div>

      {/* Actions rapides au survol : mêmes gestes que le clavier (E, S, U). */}
      <div className={cn("absolute right-2 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded-lg bg-elevated p-0.5 shadow-card group-hover:flex", selected && "bg-bg")}>
        <IconButton label="Archiver" kbd="E" onClick={(e) => { e.stopPropagation(); onArchive(t.id); }}>
          <Archive size={15} />
        </IconButton>
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
    ? "Essayez d'autres mots-clés : la recherche ignore les accents et accepte les débuts de mots."
    : inboxZero
      ? "Tout est traité. Profitez-en."
      : "Les conversations de cette vue apparaîtront ici.";
  return (
    <div className="anim-fade-up flex h-full flex-col items-center justify-center px-10 text-center">
      <span className={cn("mb-4 inline-flex h-14 w-14 items-center justify-center rounded-2xl", inboxZero ? "bg-success/10 text-success" : "bg-hover text-faint")}>
        <Icon size={26} strokeWidth={1.6} />
      </span>
      <p className="text-[14px] font-semibold">{title}</p>
      <p className="mt-1 max-w-[260px] text-[12.5px] text-muted">{hint}</p>
    </div>
  );
}

export function ThreadList() {
  const { view, category, query, selectedId, density, previews, threadsLoading, hasMore, threads, hiddenIds, setQuery, setCategory, loadMore, select, archive, toggleStar, toggleRead } = useMail(
    useShallow((s) => ({ view: s.view, category: s.category, query: s.query, selectedId: s.selectedId, density: s.density, previews: s.previews, threadsLoading: s.threadsLoading, hasMore: s.hasMore, threads: s.threads, hiddenIds: s.hiddenIds, setQuery: s.setQuery, setCategory: s.setCategory, loadMore: s.loadMore, select: s.select, archive: s.archive, toggleStar: s.toggleStar, toggleRead: s.toggleRead })),
  );
  const scroller = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const sentinel = useRef<HTMLDivElement>(null);
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
    const out: ({ kind: "h"; label: string } | { kind: "t"; t: ThreadSummary })[] = [];
    let last = "";
    for (const t of list) {
      const g = groupLabel(t.lastMessageAt);
      if (g !== last) {
        out.push({ kind: "h", label: g });
        last = g;
      }
      out.push({ kind: "t", t });
    }
    return out;
  }, [list]);

  // Garde la conversation sélectionnée visible (navigation clavier).
  useEffect(() => {
    if (selectedId !== null) scroller.current?.querySelector(`[data-thread="${selectedId}"]`)?.scrollIntoView({ block: "nearest" });
  }, [selectedId]);

  // Pagination infinie.
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !hasMore) return;
    const io = new IntersectionObserver((e) => e[0].isIntersecting && void loadMore(), { root: scroller.current, rootMargin: "240px" });
    io.observe(el);
    return () => io.disconnect();
  }, [hasMore, rows.length, loadMore]);

  // « / » focalise la recherche (événement émis par le gestionnaire clavier global).
  useEffect(() => {
    const focus = () => searchRef.current?.focus();
    window.addEventListener("envelope:focus-search", focus);
    return () => window.removeEventListener("envelope:focus-search", focus);
  }, []);

  const compact = density === "compact";
  return (
    <section className="flex h-full min-h-0 flex-col border-r border-border bg-bg-subtle" aria-label="Conversations">
      <header {...dragRegion} className="shrink-0 px-4 pt-[14px]">
        <div {...dragRegion} className="flex h-[30px] items-baseline gap-2">
          <h1 {...dragRegion} className="pointer-events-none text-[17px] font-semibold tracking-tight">{query.trim() ? "Recherche" : viewTitle(view)}</h1>
          {list.length > 0 && <span {...dragRegion} className="tnum pointer-events-none text-[12px] text-faint">{list.length}</span>}
        </div>

        <label className="mt-2.5 flex h-8 items-center gap-2 rounded-lg border border-border bg-bg px-2.5 text-muted transition-colors focus-within:border-accent/60 focus-within:ring-[3px] focus-within:ring-accent-soft">
          <Search size={14} className="shrink-0" />
          <input
            ref={searchRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setQuery("");
                e.currentTarget.blur();
              }
            }}
            placeholder="Rechercher dans les e-mails"
            aria-label="Rechercher"
            className="selectable min-w-0 flex-1 bg-transparent text-[13px] text-fg outline-none placeholder:text-faint"
          />
          {query ? (
            <button aria-label="Effacer la recherche" onClick={() => setQuery("")} className="press rounded p-0.5 hover:bg-hover">
              <X size={13} />
            </button>
          ) : (
            <Kbd>/</Kbd>
          )}
        </label>

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
          <>
            {rows.map((r) =>
              r.kind === "h" ? (
                <div key={`h-${r.label}`} className="sticky top-0 z-10 bg-bg-subtle/90 px-5 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-faint backdrop-blur-md">
                  {r.label}
                </div>
              ) : (
                <Row key={r.t.id} t={r.t} selected={r.t.id === selectedId} compact={compact} previews={previews} onSelect={select} onArchive={archive} onStar={toggleStar} onToggleRead={toggleRead} />
              ),
            )}
            <div ref={sentinel} className="h-px" />
          </>
        )}
      </div>
    </section>
  );
}
