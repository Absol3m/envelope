import { ChevronDown, ChevronUp, Forward, Mail, MailOpen, Reply, ReplyAll, Star, Trash2 } from "lucide-react";
import { useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import { useComposer } from "@/lib/composer";
import { dragRegion } from "@/lib/platform";
import { actionView, useMail, visibleThreads } from "@/lib/store";
import { DESTINATIONS, destinationsFor, isPurgeView } from "@/lib/views";
import { SearchBox } from "./SearchBox";
import { IconButton } from "./ui/primitives";

/** Barre d'outils de la fenêtre : actions sur la conversation ouverte à gauche, recherche en haut à droite.
 *  Sous macOS elle double la barre de titre (zone de glisser) ; ailleurs elle se place sous la barre de titre native. */
export function TopBar() {
  const { backend, accountFilter, query, setQuery, selectedId, checked, threads, folders, hiddenIds, category, view, moveTo, deleteForever, toggleStar, toggleRead, move } = useMail(
    useShallow((s) => ({ backend: s.backend, accountFilter: s.accountFilter, query: s.query, setQuery: s.setQuery, selectedId: s.selectedId, checked: s.checked, threads: s.threads, folders: s.folders, hiddenIds: s.hiddenIds, category: s.category, view: s.view, moveTo: s.moveTo, deleteForever: s.deleteForever, toggleStar: s.toggleStar, toggleRead: s.toggleRead, move: s.move })),
  );
  const summary = checked.size > 1 ? undefined : threads.find((t) => t.id === selectedId);
  const list = useMemo(() => visibleThreads({ threads, hiddenIds, category, view, query }), [threads, hiddenIds, category, view, query]);
  const index = list.findIndex((t) => t.id === selectedId);
  const unread = (summary?.unreadCount ?? 0) > 0;
  const place = summary ? actionView({ view, query, threads, folders }, [summary.id]) : view;

  return (
    <div {...dragRegion} className="col-span-2 col-start-2 flex h-[46px] shrink-0 items-center gap-0.5 border-b border-border bg-bg/80 px-3 backdrop-blur-md">
      {summary && (
        <>
          <IconButton label="Répondre" kbd="R" onClick={() => useComposer.getState().respond("reply")}><Reply size={16} /></IconButton>
          <IconButton label="Répondre à tous" kbd="A" onClick={() => useComposer.getState().respond("replyAll")}><ReplyAll size={16} /></IconButton>
          <IconButton label="Transférer" kbd="F" onClick={() => useComposer.getState().respond("forward")}><Forward size={16} /></IconButton>
          {destinationsFor(place).map((d) => {
            const { label, icon: Icon, key } = DESTINATIONS[d];
            return <IconButton key={d} label={label} kbd={key} onClick={() => moveTo(summary.id, d)}><Icon size={16} /></IconButton>;
          })}
          {isPurgeView(place) && <IconButton label="Supprimer définitivement" kbd="⌫" onClick={() => void deleteForever(summary.id)}><Trash2 size={16} className="text-danger" /></IconButton>}
          <IconButton label={summary.isStarred ? "Ne plus suivre" : "Suivre"} kbd="S" onClick={() => toggleStar(summary.id)}><Star size={16} className={summary.isStarred ? "fill-star text-star" : ""} /></IconButton>
          <IconButton label={unread ? "Marquer comme lu" : "Marquer comme non lu"} kbd="U" onClick={() => toggleRead(summary.id)}>{unread ? <MailOpen size={16} /> : <Mail size={16} />}</IconButton>
        </>
      )}
      <span {...dragRegion} className="min-w-3 flex-1 self-stretch" />
      {summary && index >= 0 && <span className="tnum mr-1 hidden text-[12px] text-muted min-[1280px]:inline">{index + 1} sur {list.length}</span>}
      {summary && (
        <>
          <IconButton label="Précédente" kbd="K" disabled={index <= 0} onClick={() => move(-1)}><ChevronUp size={16} /></IconButton>
          <IconButton label="Suivante" kbd="J" disabled={index < 0 || index >= list.length - 1} onClick={() => move(1)}><ChevronDown size={16} /></IconButton>
        </>
      )}
      {/* Le champ s'élargit au focus : de la place pour les opérateurs et leurs valeurs. */}
      <div className="ml-2 w-[220px] shrink-0 transition-[width] duration-200 focus-within:w-[320px]">
        <SearchBox query={query} setQuery={setQuery} backend={backend} accountId={accountFilter} />
      </div>
    </div>
  );
}
