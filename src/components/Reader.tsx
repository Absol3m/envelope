import { ChevronDown, ChevronUp, File, FileArchive, FileImage, FileText, Forward, ImageOff, Mail, MailOpen, Paperclip, Reply, ReplyAll, Star, Trash2 } from "lucide-react";
import { DESTINATIONS, destinationsFor, isPurgeView } from "@/lib/views";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Account, DetailAttachment, DetailMessage, ThreadDetail, ThreadSummary } from "@/lib/api";
import { displayName, fileSize, fullDate, listDate } from "@/lib/format";
import { prepareHtml, trustedSenders, trustSender } from "@/lib/sanitize";
import { useMail, visibleThreads } from "@/lib/store";
import { cn, openExternal } from "@/lib/utils";
import { useComposer } from "@/lib/composer";
import { Avatar, IconButton, Kbd } from "./ui/primitives";
import { dragRegion, shortcut, suppressWebviewContextMenu } from "@/lib/platform";

// ───────────── Corps du message ─────────────

/** Iframe sandbox (aucun script) : le HTML d'un expéditeur n'a aucun accès à l'application. */
function HtmlFrame({ srcDoc }: { srcDoc: string }) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(72);

  useEffect(() => {
    const frame = ref.current;
    if (!frame) return;
    let ro: ResizeObserver | undefined;
    const attach = () => {
      const doc = frame.contentDocument;
      if (!doc?.documentElement) return;
      const measure = () => setHeight(Math.ceil(doc.documentElement.scrollHeight));
      measure();
      ro?.disconnect();
      ro = new ResizeObserver(measure);
      ro.observe(doc.documentElement);
      suppressWebviewContextMenu(doc);
      // Les liens s'ouvrent dans le navigateur système, jamais dans l'application.
      doc.addEventListener("click", (e) => {
        const a = (e.target as Element | null)?.closest?.("a[href]");
        if (a) {
          e.preventDefault();
          void openExternal(a.getAttribute("href") ?? "");
        }
      });
    };
    frame.addEventListener("load", attach);
    if (frame.contentDocument?.readyState === "complete") attach();
    return () => {
      frame.removeEventListener("load", attach);
      ro?.disconnect();
    };
  }, [srcDoc]);

  return <iframe ref={ref} title="Contenu du message" sandbox="allow-same-origin" srcDoc={srcDoc} style={{ height }} className="block w-full border-0 bg-white" />;
}

const QUOTE_START = /^(>|le .{5,120} a écrit\s*:|on .{5,120} wrote:|-{2,}\s*(message d'origine|original message))/i;

function Linkified({ text }: { text: string }) {
  const parts = text.split(/(https?:\/\/[^\s<>"]+[^\s<>".,;:!?)])/g);
  return (
    <>
      {parts.map((p, i) =>
        i % 2 === 1 ? (
          <a key={i} href={p} onClick={(e) => { e.preventDefault(); void openExternal(p); }} className="text-accent underline decoration-accent/30 underline-offset-2 hover:decoration-accent">
            {p}
          </a>
        ) : (
          <Fragment key={i}>{p}</Fragment>
        ),
      )}
    </>
  );
}

/** Texte brut : liens cliquables, citations repliées derrière « ··· » (comme un fil de discussion). */
function TextBody({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const q = lines.findIndex((l, i) => i > 0 && QUOTE_START.test(l.trim()));
  const main = (q > 0 ? lines.slice(0, q) : lines).join("\n").trimEnd();
  const quoted = q > 0 ? lines.slice(q).join("\n") : "";
  return (
    <div className="selectable whitespace-pre-wrap break-words px-1 text-[14px] leading-[1.65] text-fg">
      <Linkified text={main} />
      {quoted && (
        <>
          <button onClick={() => setOpen(!open)} aria-expanded={open} className="press mt-2 block rounded-md bg-hover px-2 py-0.5 text-[12px] font-medium tracking-widest text-muted hover:bg-active" title={open ? "Masquer la citation" : "Afficher la citation"}>
            ···
          </button>
          {open && <div className="anim-fade-in mt-2 border-l-2 border-border-strong pl-3 text-[13px] text-muted"><Linkified text={quoted} /></div>}
        </>
      )}
    </div>
  );
}

function MessageBody({ m }: { m: DetailMessage }) {
  const backend = useMail((s) => s.backend);
  const [allow, setAllow] = useState(() => trustedSenders().has(m.fromAddr.toLowerCase()));
  const [cid, setCid] = useState<Record<string, string>>();
  const html = m.bodyHtml;
  const needsCid = !!html && /cid:/i.test(html);

  useEffect(() => {
    if (needsCid) backend.inlineImages(m.id).then(setCid, () => setCid({}));
  }, [needsCid, m.id, backend]);

  // Une seule sanitisation, une fois les images inline connues (sinon l'iframe se rechargerait).
  const ready = !needsCid || cid !== undefined;
  const prepared = useMemo(() => (html && ready ? prepareHtml(html, { allowRemote: allow, cid }) : null), [html, ready, allow, cid]);

  if (html && !ready) return <div className="skeleton h-24 rounded-xl" aria-busy />;

  if (prepared) {
    return (
      <div>
        {prepared.blocked > 0 && (
          <div className="anim-fade-in mb-2 flex items-center gap-2.5 rounded-lg bg-warn/10 px-3 py-2 text-[12.5px] text-fg">
            <ImageOff size={15} className="shrink-0 text-warn" />
            <span className="flex-1">Images distantes bloquées pour protéger votre vie privée.</span>
            <button onClick={() => setAllow(true)} className="press rounded-md bg-bg px-2 py-1 font-medium shadow-card hover:bg-hover">Afficher</button>
            <button onClick={() => { trustSender(m.fromAddr); setAllow(true); }} className="press rounded-md px-2 py-1 font-medium text-muted hover:bg-hover hover:text-fg">Toujours pour cet expéditeur</button>
          </div>
        )}
        <div className="overflow-hidden rounded-xl shadow-card"><HtmlFrame srcDoc={prepared.srcDoc} /></div>
      </div>
    );
  }
  return <TextBody text={m.bodyText ?? "(message vide)"} />;
}

// ───────────── Pièces jointes ─────────────

function attachmentIcon(mime: string) {
  if (mime.startsWith("image/")) return FileImage;
  if (mime === "application/pdf" || mime.startsWith("text/")) return FileText;
  if (/zip|compressed|tar|rar|7z/.test(mime)) return FileArchive;
  return File;
}

function Attachments({ items }: { items: DetailAttachment[] }) {
  const { backend, toast } = useMail(useShallow((s) => ({ backend: s.backend, toast: s.toast })));
  if (items.length === 0) return null;
  return (
    <div className="mt-3 flex flex-wrap gap-2">
      {items.map((a) => {
        const Icon = attachmentIcon(a.mimeType);
        return (
          <button
            key={a.id}
            onClick={() => backend.openAttachment(a.id).catch((e) => toast({ message: `Ouverture impossible — ${String(e)}`, tone: "error" }))}
            className="press group flex max-w-[260px] items-center gap-2.5 rounded-lg bg-bg py-1.5 pl-2 pr-3 text-left shadow-card hover:bg-hover"
          >
            <span className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-accent-soft text-accent"><Icon size={16} /></span>
            <span className="min-w-0">
              <span className="block truncate text-[12.5px] font-medium">{a.filename ?? "pièce jointe"}</span>
              <span className="tnum block text-[11px] text-muted">{fileSize(a.size)}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}

// ───────────── Carte de message ─────────────

function recipients(m: DetailMessage, me: string): string {
  const names = m.to.map((r) => (r.address.toLowerCase() === me.toLowerCase() ? "moi" : displayName(r.name, r.address).split(" ")[0]));
  return names.length ? `à ${names.join(", ")}` : "";
}

function MessageCard({ m, atts, me, expanded, onToggle }: { m: DetailMessage; atts: DetailAttachment[]; me: string; expanded: boolean; onToggle(): void }) {
  const [details, setDetails] = useState(false);
  const sender = displayName(m.fromName, m.fromAddr);
  const preview = (m.bodyText ?? "").split(/\n/).find((l) => l.trim() && !l.startsWith(">"))?.trim() ?? "";
  const files = atts.filter((a) => !a.isInline || a.filename);

  if (!expanded) {
    return (
      <button onClick={onToggle} className="press flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left hover:bg-hover" aria-expanded={false}>
        <Avatar name={m.fromName} address={m.fromAddr} size={28} />
        <span className="w-32 shrink-0 truncate text-[13px] font-medium">{sender}</span>
        <span className="min-w-0 flex-1 truncate text-[13px] text-muted">{preview || "(message sans texte)"}</span>
        {files.length > 0 && <Paperclip size={13} className="shrink-0 text-faint" />}
        <span className="tnum shrink-0 text-[12px] text-muted">{listDate(m.date)}</span>
      </button>
    );
  }
  return (
    <article className="anim-fade-up rounded-2xl bg-bg px-4 pb-4 pt-3.5 shadow-card" aria-label={`Message de ${sender}`}>
      <header className="flex items-start gap-3">
        <Avatar name={m.fromName} address={m.fromAddr} size={36} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <button onClick={onToggle} className="truncate text-[14px] font-semibold hover:underline" title="Replier">{sender}</button>
            <span className="selectable truncate text-[12px] text-muted">&lt;{m.fromAddr}&gt;</span>
          </div>
          <button onClick={() => setDetails(!details)} className="flex items-center gap-1 text-[12px] text-muted hover:text-fg" aria-expanded={details}>
            {recipients(m, me) || "—"}
            <ChevronDown size={12} className={cn("transition-transform", details && "rotate-180")} />
          </button>
        </div>
        <time dateTime={m.date} title={fullDate(m.date)} className="tnum shrink-0 pt-0.5 text-[12px] text-muted">{listDate(m.date)}</time>
      </header>

      {details && (
        <dl className="selectable anim-fade-in mt-3 grid grid-cols-[52px_1fr] gap-x-3 gap-y-1 rounded-lg bg-bg-subtle px-3 py-2 text-[12px]">
          <dt className="text-faint">De</dt><dd>{m.fromName ? `${m.fromName} <${m.fromAddr}>` : m.fromAddr}</dd>
          <dt className="text-faint">À</dt><dd>{m.to.map((r) => r.address).join(", ") || "—"}</dd>
          {m.cc.length > 0 && (<><dt className="text-faint">Cc</dt><dd>{m.cc.map((r) => r.address).join(", ")}</dd></>)}
          <dt className="text-faint">Date</dt><dd className="first-letter:uppercase">{fullDate(m.date)}</dd>
        </dl>
      )}

      <div className="mt-3.5"><MessageBody m={m} /></div>
      <Attachments items={files} />
    </article>
  );
}

// ───────────── Conversation ─────────────

function Skeleton({ summary }: { summary?: ThreadSummary }) {
  return (
    <div className="mx-auto max-w-[780px] px-8 pt-8" aria-busy>
      <h2 className="text-[22px] font-semibold tracking-tight">{summary?.subject ?? <span className="skeleton block h-7 w-2/3" />}</h2>
      <div className="mt-8 space-y-3">
        {[0.9, 0.7].map((o, i) => (
          <div key={i} className="rounded-2xl bg-bg p-4 shadow-card" style={{ opacity: o }}>
            <div className="flex gap-3"><div className="skeleton h-9 w-9 rounded-full" /><div className="flex-1 space-y-2"><div className="skeleton h-3 w-40" /><div className="skeleton h-3 w-24" /></div></div>
            <div className="mt-4 space-y-2"><div className="skeleton h-3 w-full" /><div className="skeleton h-3 w-11/12" /><div className="skeleton h-3 w-3/5" /></div>
          </div>
        ))}
      </div>
    </div>
  );
}

function ThreadView({ d, account }: { d: ThreadDetail; account?: Account }) {
  const accounts = useMail((s) => s.accounts);
  const initial = useMemo(() => {
    const open = new Set<number>(d.messages.filter((m) => !m.isRead).map((m) => m.id));
    const last = d.messages[d.messages.length - 1];
    if (last) open.add(last.id); // le dernier message est toujours ouvert
    return open;
  }, [d.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const [expanded, setExpanded] = useState(initial);
  const allOpen = expanded.size >= d.messages.length;
  const toggle = (id: number) => setExpanded((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const files = d.attachments.filter((a) => !a.isInline || a.filename).length;

  return (
    <div className="mx-auto max-w-[780px] px-8 pb-24 pt-7">
      <div className="anim-fade-up">
        <h2 className="selectable text-[22px] font-semibold leading-tight tracking-tight">{d.subject || "(sans objet)"}</h2>
        <div className="mt-2 flex flex-wrap items-center gap-2 text-[12px] text-muted">
          <span className="tnum">{d.messages.length} message{d.messages.length > 1 ? "s" : ""}</span>
          {files > 0 && <span className="inline-flex items-center gap-1"><Paperclip size={12} />{files}</span>}
          {accounts.length > 1 && account && <span className="rounded-full bg-hover px-2 py-0.5 text-[11px] font-medium">{account.email}</span>}
          {d.messages.length > 1 && (
            <button onClick={() => setExpanded(allOpen ? new Set([d.messages[d.messages.length - 1].id]) : new Set(d.messages.map((m) => m.id)))} className="press ml-auto rounded-md px-2 py-0.5 font-medium hover:bg-hover hover:text-fg">
              {allOpen ? "Tout replier" : "Tout déplier"}
            </button>
          )}
        </div>
      </div>
      <div className="mt-6 space-y-2">
        {d.messages.map((m) => (
          <MessageCard key={m.id} m={m} atts={d.attachments.filter((a) => a.messageId === m.id)} me={account?.email ?? ""} expanded={expanded.has(m.id)} onToggle={() => toggle(m.id)} />
        ))}
      </div>
    </div>
  );
}

function EmptyReader() {
  const items: [string, string][] = [["C", "Nouveau message"], ["J / K", "Naviguer"], [DESTINATIONS.archive.key, DESTINATIONS.archive.label], [shortcut("mod+k"), "Palette de commandes"]];
  return (
    <div className="anim-fade-in flex h-full flex-col items-center justify-center text-center">
      <span className="mb-5 inline-flex h-16 w-16 items-center justify-center rounded-3xl bg-gradient-to-br from-accent/15 to-[#8b5cf6]/15 text-accent"><Mail size={28} strokeWidth={1.5} /></span>
      <p className="text-[15px] font-semibold">Sélectionnez une conversation</p>
      <p className="mt-1 text-[13px] text-muted">Sélectionnez une conversation. Les raccourcis sont là si vous les voulez : <Kbd>?</Kbd>.</p>
      <div className="mt-6 flex gap-5 text-[12px] text-muted">
        {items.map(([k, label]) => (
          <span key={k} className="flex items-center gap-1.5"><Kbd>{k}</Kbd>{label}</span>
        ))}
      </div>
    </div>
  );
}

export function Reader() {
  const { selectedId, details, threads, accounts, hiddenIds, category, view, query, moveTo, deleteForever, toggleStar, toggleRead, move } = useMail(
    useShallow((s) => ({ selectedId: s.selectedId, details: s.details, threads: s.threads, accounts: s.accounts, hiddenIds: s.hiddenIds, category: s.category, view: s.view, query: s.query, moveTo: s.moveTo, deleteForever: s.deleteForever, toggleStar: s.toggleStar, toggleRead: s.toggleRead, move: s.move })),
  );
  const summary = threads.find((t) => t.id === selectedId);
  const d = selectedId === null ? undefined : details[selectedId];
  const list = useMemo(() => visibleThreads({ threads, hiddenIds, category, view, query }), [threads, hiddenIds, category, view, query]);
  const index = list.findIndex((t) => t.id === selectedId);
  const unread = (summary?.unreadCount ?? 0) > 0;
  const account = accounts.find((a) => a.id === (d?.accountId ?? summary?.accountId));

  return (
    <main className="flex h-full min-h-0 min-w-0 flex-col bg-bg-subtle" aria-label="Lecture">
      <div {...dragRegion} className="flex h-[46px] shrink-0 items-center gap-0.5 border-b border-border bg-bg/80 px-3 backdrop-blur-md">
        {summary ? (
          <>
            <IconButton label="Répondre" kbd="R" onClick={() => useComposer.getState().respond("reply")}><Reply size={16} /></IconButton>
            <IconButton label="Répondre à tous" kbd="A" onClick={() => useComposer.getState().respond("replyAll")}><ReplyAll size={16} /></IconButton>
            <IconButton label="Transférer" kbd="F" onClick={() => useComposer.getState().respond("forward")}><Forward size={16} /></IconButton>
            {destinationsFor(view).map((d) => {
              const { label, icon: Icon, key } = DESTINATIONS[d];
              return <IconButton key={d} label={label} kbd={key} onClick={() => moveTo(summary.id, d)}><Icon size={16} /></IconButton>;
            })}
            {isPurgeView(view) && <IconButton label="Supprimer définitivement" kbd="⌫" onClick={() => void deleteForever(summary.id)}><Trash2 size={16} className="text-danger" /></IconButton>}
            <IconButton label={summary.isStarred ? "Ne plus suivre" : "Suivre"} kbd="S" onClick={() => toggleStar(summary.id)}><Star size={16} className={summary.isStarred ? "fill-star text-star" : ""} /></IconButton>
            <IconButton label={unread ? "Marquer comme lu" : "Marquer comme non lu"} kbd="U" onClick={() => toggleRead(summary.id)}>{unread ? <MailOpen size={16} /> : <Mail size={16} />}</IconButton>
            <span {...dragRegion} className="flex-1" />
            {index >= 0 && <span className="tnum mr-1 text-[12px] text-muted">{index + 1} sur {list.length}</span>}
            <IconButton label="Précédente" kbd="K" disabled={index <= 0} onClick={() => move(-1)}><ChevronUp size={16} /></IconButton>
            <IconButton label="Suivante" kbd="J" disabled={index < 0 || index >= list.length - 1} onClick={() => move(1)}><ChevronDown size={16} /></IconButton>
          </>
        ) : (
          <span {...dragRegion} className="flex-1" />
        )}
      </div>

      <div className="selectable min-h-0 flex-1 overflow-y-auto" key={selectedId ?? "none"}>
        {selectedId === null ? <EmptyReader /> : d ? <ThreadView d={d} account={account} /> : <Skeleton summary={summary} />}
      </div>
    </main>
  );
}

