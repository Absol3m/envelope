import { ChevronDown, Download, File, FileArchive, FileImage, FileText, ImageOff, Mail, MailOpen, Paperclip, ShieldAlert, ShieldCheck, Star, Trash2 } from "lucide-react";
import { DESTINATIONS, destinationsFor, isPurgeView } from "@/lib/views";
import { Fragment, useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Account, DetailAttachment, DetailMessage, ThreadDetail, ThreadSummary } from "@/lib/api";
import { displayName, fileSize, fullDate, listDate } from "@/lib/format";
import { openLink } from "@/lib/links";
import { prepareHtml, trustedSenders, trustSender } from "@/lib/sanitize";
import { actionView, useMail } from "@/lib/store";
import { cn, openExternal } from "@/lib/utils";
import { useComposer } from "@/lib/composer";
import { Avatar, btnPrimary, Kbd } from "./ui/primitives";
import { BellOff } from "lucide-react";
import { isLinux, shortcut, suppressWebviewContextMenu } from "@/lib/platform";
import { t as tr } from "@/lib/i18n";
import { AttachmentPreview, InlineImage } from "./AttachmentPreview";

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
          // Destination douteuse (texte trompeur, adresse numérique, lettres imitées) : confirmation avant ouverture.
          void openLink(a.getAttribute("href") ?? "", a.textContent ?? "");
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

  return <iframe ref={ref} title={tr("reader:content")} sandbox="allow-same-origin" srcDoc={srcDoc} style={{ height }} className="block w-full border-0 bg-white" />;
}

const QUOTE_START = /^(>|le .{5,120} a écrit\s*:|on .{5,120} wrote:|-{2,}\s*(message d'origine|original message))/i;

function Linkified({ text }: { text: string }) {
  const parts = text.split(/(https?:\/\/[^\s<>"]+[^\s<>".,;:!?)])/g);
  return (
    <>
      {parts.map((p, i) =>
        i % 2 === 1 ? (
          <a key={i} href={p} onClick={(e) => { e.preventDefault(); void openLink(p, p); }} className="text-accent underline decoration-accent/30 underline-offset-2 hover:decoration-accent">
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
          <button onClick={() => setOpen(!open)} aria-expanded={open} className="press mt-2 block rounded-md bg-hover px-2 py-0.5 text-[12px] font-medium tracking-widest text-muted hover:bg-active" title={open ? tr("reader:hideQuote") : tr("reader:showQuote")}>
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
            <span className="flex-1">{tr("reader:imagesBlocked")}</span>
            <button onClick={() => setAllow(true)} className="press rounded-md bg-bg px-2 py-1 font-medium shadow-card hover:bg-hover">{tr("reader:show")}</button>
            <button onClick={() => { trustSender(m.fromAddr); setAllow(true); }} className="press rounded-md px-2 py-1 font-medium text-muted hover:bg-hover hover:text-fg">{tr("reader:alwaysShow")}</button>
          </div>
        )}
        {prepared.trackers > 0 && (
          <p className="mb-2 flex items-center gap-1.5 px-1 text-[12px] text-muted"><ShieldCheck size={13} className="shrink-0 text-success" />{tr("reader:trackers", { count: prepared.trackers })}</p>
        )}
        <HtmlFrame srcDoc={prepared.srcDoc} />
      </div>
    );
  }
  return <TextBody text={m.bodyText ?? tr("reader:empty")} />;
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
  const [preview, setPreview] = useState<DetailAttachment | null>(null);
  const [unshown, setUnshown] = useState<ReadonlySet<number>>(new Set()); // images trop grosses ou illisibles : gardées en pièce jointe
  if (items.length === 0) return null;
  // Les images s'affichent dans le fil, sous le message (celles déjà dans le corps, `cid:`, n'y sont pas répétées).
  const isShown = (a: DetailAttachment) => a.previewable && a.mimeType.startsWith("image/") && !a.isInline && !unshown.has(a.id);
  // WebKitGTK (Linux) n'a pas de lecteur PDF : le PDF s'ouvre dans l'application du système.
  const inApp = (a: DetailAttachment) => a.previewable && !(isLinux && a.mimeType.toLowerCase() === "application/pdf");
  const shown = items.filter(isShown);
  const chips = items.filter((a) => !isShown(a));
  const fail = (key: "reader:openFailed" | "reader:saveFailed") => (e: unknown) => toast({ message: tr(key, { error: String(e) }), tone: "error" });
  const open = (a: DetailAttachment) => backend.openAttachment(a.id).catch(fail("reader:openFailed"));
  const save = (list: DetailAttachment[]) => backend.saveAttachments(list.map((a) => a.id)).then((n) => n > 0 && toast({ message: tr("reader:saved", { count: n }) }), fail("reader:saveFailed"));
  return (
    <div className="mt-3 flex flex-col items-start gap-2">
      {shown.map((a) => <InlineImage key={a.id} item={a} onOpen={() => setPreview(a)} onSave={() => void save([a])} onFail={() => setUnshown((s) => new Set(s).add(a.id))} />)}
      {(chips.length > 0 || items.length > 1) && <div className="flex flex-wrap items-center gap-2">
      {chips.map((a) => {
        const Icon = attachmentIcon(a.mimeType);
        return (
          <div key={a.id} className="group flex max-w-[300px] items-center rounded-lg bg-bg shadow-card">
            <button onClick={() => (inApp(a) ? setPreview(a) : void open(a))} className="press flex min-w-0 items-center gap-2.5 rounded-l-lg py-1.5 pl-2 pr-2 text-left hover:bg-hover">
              <span className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-accent-soft text-accent"><Icon size={16} /></span>
              <span className="min-w-0">
                <span className="block truncate text-[12.5px] font-medium">{a.filename ?? tr("reader:attachment")}</span>
                <span className="tnum block text-[11px] text-muted">{fileSize(a.size)}</span>
              </span>
            </button>
            <button aria-label={tr("reader:save")} title={tr("reader:save")} onClick={() => void save([a])} className="press self-stretch rounded-r-lg px-2 text-muted hover:bg-hover hover:text-fg"><Download size={14} /></button>
          </div>
        );
      })}
      {items.length > 1 && <button onClick={() => void save(items)} className="press rounded-md px-2 py-1 text-[12px] text-muted hover:bg-hover hover:text-fg">{tr("reader:saveAll")}</button>}
      </div>}
      {preview && <AttachmentPreview item={preview} onClose={() => setPreview(null)} onSave={() => void save([preview])} onOpen={() => void open(preview)} />}
    </div>
  );
}

// ───────────── Carte de message ─────────────

function recipients(m: DetailMessage, me: string): string {
  const names = m.to.map((r) => (r.address.toLowerCase() === me.toLowerCase() ? tr("reader:me") : displayName(r.name, r.address).split(" ")[0]));
  return names.length ? tr("reader:to", { names: names.join(", ") }) : "";
}

const authFailText = (kind: "dmarc" | "spf" | "dkim") => tr(`reader:authFail.${kind}`);

/** Surface d'un message (replié, ouvert, squelette) : fond teinté par un voile translucide, donc valable dans les deux thèmes, sans ombre ni liseré. */
const CARD = "rounded-[10px] bg-hover";
/** Gouttière du volet de lecture : contenu pleine largeur, collé à gauche (conversation et squelette). */
const GUTTER = "px-4 pt-5";

function MessageCard({ m, atts, me, expanded, onToggle }: { m: DetailMessage; atts: DetailAttachment[]; me: string; expanded: boolean; onToggle(): void }) {
  const [details, setDetails] = useState(false);
  const sender = displayName(m.fromName, m.fromAddr);
  const preview = (m.bodyText ?? "").split(/\n/).find((l) => l.trim() && !l.startsWith(">"))?.trim() ?? "";
  const files = atts.filter((a) => !a.isInline || a.filename);

  if (!expanded) {
    return (
      <button onClick={onToggle} className={cn(CARD, "press flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-active")} aria-expanded={false}>
        <Avatar name={m.fromName} address={m.fromAddr} size={28} />
        <span className="w-32 shrink-0 truncate text-[13px] font-medium">{sender}</span>
        <span className="min-w-0 flex-1 truncate text-[13px] text-muted">{preview || tr("reader:noText")}</span>
        {files.length > 0 && <Paperclip size={13} className="shrink-0 text-faint" />}
        <span className="tnum shrink-0 text-[12px] text-muted">{listDate(m.date)}</span>
      </button>
    );
  }
  return (
    <article className={cn(CARD, "anim-fade-up px-4 pb-4 pt-3.5")} aria-label={tr("reader:messageFrom", { sender })}>
      <header className="flex items-start gap-3">
        <Avatar name={m.fromName} address={m.fromAddr} size={36} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <button onClick={onToggle} className="truncate text-[14px] font-semibold hover:underline" title={tr("reader:collapse")}>{sender}</button>
            <span className="selectable truncate text-[12px] text-muted">&lt;{m.fromAddr}&gt;</span>
          </div>
          <button onClick={() => setDetails(!details)} className="flex items-center gap-1 text-[12px] text-muted hover:text-fg" aria-expanded={details}>
            {recipients(m, me) || "—"}
            <ChevronDown size={12} className={cn("transition-transform", details && "rotate-180")} />
          </button>
        </div>
        <time dateTime={m.date} title={fullDate(m.date)} className="tnum shrink-0 pt-0.5 text-[12px] text-muted">{listDate(m.date)}</time>
      </header>

      {m.authFail && (
        <div role="alert" className="anim-fade-in mt-3 flex items-start gap-2.5 rounded-lg bg-danger/10 px-3 py-2 text-[12.5px] text-fg">
          <ShieldAlert size={15} className="mt-px shrink-0 text-danger" />
          <span><strong className="font-semibold">{tr("reader:unverified")}</strong> {authFailText(m.authFail)} {tr("reader:unverifiedAdvice")}</span>
        </div>
      )}

      {details && (
        <dl className="selectable anim-fade-in mt-3 grid grid-cols-[52px_1fr] gap-x-3 gap-y-1 rounded-lg bg-bg-subtle px-3 py-2 text-[12px]">
          <dt className="text-faint">{tr("reader:from")}</dt><dd>{m.fromName ? `${m.fromName} <${m.fromAddr}>` : m.fromAddr}</dd>
          <dt className="text-faint">{tr("reader:toLabel")}</dt><dd>{m.to.map((r) => r.address).join(", ") || "—"}</dd>
          {m.cc.length > 0 && (<><dt className="text-faint">Cc</dt><dd>{m.cc.map((r) => r.address).join(", ")}</dd></>)}
          <dt className="text-faint">{tr("reader:date")}</dt><dd className="first-letter:uppercase">{fullDate(m.date)}</dd>
        </dl>
      )}

      <div className="mt-3 border-t border-border pt-3"><MessageBody m={m} /></div>
      <Attachments items={files} />
    </article>
  );
}

// ───────────── Conversation ─────────────

function Skeleton({ summary }: { summary?: ThreadSummary }) {
  return (
    <div className={GUTTER} aria-busy>
      <h2 className="text-[22px] font-semibold tracking-tight">{summary?.subject ?? <span className="skeleton block h-7 w-2/3" />}</h2>
      <div className="mt-8 space-y-3">
        {[0.9, 0.7].map((o, i) => (
          <div key={i} className={cn(CARD, "p-4")} style={{ opacity: o }}>
            <div className="flex gap-3"><div className="skeleton h-9 w-9 rounded-full" /><div className="flex-1 space-y-2"><div className="skeleton h-3 w-40" /><div className="skeleton h-3 w-24" /></div></div>
            <div className="mt-4 space-y-2"><div className="skeleton h-3 w-full" /><div className="skeleton h-3 w-11/12" /><div className="skeleton h-3 w-3/5" /></div>
          </div>
        ))}
      </div>
    </div>
  );
}

/** « Se désabonner » d'une liste (en-têtes List-Unsubscribe du dernier message qui en propose un). */
function UnsubscribeButton({ m }: { m: DetailMessage }) {
  const { backend, toast } = useMail(useShallow((s) => ({ backend: s.backend, toast: s.toast })));
  const [busy, setBusy] = useState(false);
  const u = m.unsubscribe;
  if (!u) return null;
  const sender = displayName(m.fromName, m.fromAddr);
  const run = async () => {
    // Un simple lien (sans POST ni adresse) s'ouvre dans le navigateur : l'utilisateur termine sur la page.
    if (!u.oneClick && !u.mailto && u.url) return void openExternal(u.url);
    if (!(await backend.confirm(tr("reader:unsubscribeConfirm", { sender }), tr("reader:unsubscribe")))) return;
    setBusy(true);
    try {
      await backend.unsubscribe(m.id);
      toast({ message: u.oneClick ? tr("reader:unsubscribed", { sender }) : tr("reader:unsubscribeRequested", { sender }) });
    } catch (e) {
      toast({ message: tr("reader:unsubscribeFailed", { error: String(e) }), tone: "error", duration: 6000 });
    } finally {
      setBusy(false);
    }
  };
  return (
    <button onClick={() => void run()} disabled={busy} className="press inline-flex items-center gap-1 rounded-md px-2 py-0.5 font-medium hover:bg-hover hover:text-fg disabled:opacity-50">
      <BellOff size={12} />{tr("reader:unsubscribe")}
    </button>
  );
}

function ThreadView({ d, account }: { d: ThreadDetail; account?: Account }) {
  const accounts = useMail((s) => s.accounts);
  const expandThreads = useMail((s) => s.expandThreads);
  const allIds = () => new Set(d.messages.map((m) => m.id));
  // État de départ seulement (le lecteur est remonté à chaque conversation) : le réglage ne rouvre pas ce que l'utilisateur a replié.
  const [expanded, setExpanded] = useState(() => {
    if (expandThreads) return allIds(); // comme Mail : tout est déplié
    const open = new Set(d.messages.filter((m) => !m.isRead).map((m) => m.id));
    const last = d.messages[d.messages.length - 1];
    if (last) open.add(last.id); // le dernier message est toujours ouvert
    return open;
  });
  const allOpen = expanded.size >= d.messages.length;
  const toggle = (id: number) => setExpanded((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const files = d.attachments.filter((a) => !a.isInline || a.filename).length;
  const unsubscribable = [...d.messages].reverse().find((m) => m.unsubscribe);
  // Conversation qui contient un brouillon (dossier Brouillons) : on propose de le rouvrir.
  const draftFolders = useMail((s) => s.folders);
  const hasDraft = useMemo(() => {
    const ids = new Set(draftFolders.filter((f) => f.role === "drafts").map((f) => f.id));
    return d.messages.some((m) => ids.has(m.folderId));
  }, [draftFolders, d.messages]);

  return (
    <div className={cn(GUTTER, "pb-24")}>
      {hasDraft && (
        <div className="anim-fade-up mb-4 flex items-center gap-3 rounded-[10px] bg-accent-soft px-4 py-2.5 text-[13px]">
          <span className="flex-1">{tr("reader:hasDraft")}</span>
          <button className={btnPrimary} onClick={() => void useComposer.getState().openDraft(d.id)}>{tr("reader:editDraft")}</button>
        </div>
      )}
      <div className="anim-fade-up px-1">
        <h2 className="selectable text-[22px] font-semibold leading-tight tracking-tight">{d.subject || tr("list:noSubject")}</h2>
        <div className="mt-2 flex flex-wrap items-center gap-2 text-[12px] text-muted">
          <span className="tnum">{tr("reader:messageCount", { count: d.messages.length })}</span>
          {files > 0 && <span className="inline-flex items-center gap-1"><Paperclip size={12} />{files}</span>}
          {unsubscribable && <UnsubscribeButton m={unsubscribable} />}
          {accounts.length > 1 && account && <span className="rounded-full bg-hover px-2 py-0.5 text-[11px] font-medium">{account.email}</span>}
          {d.messages.length > 1 && (
            <button onClick={() => setExpanded(allOpen ? new Set([d.messages[d.messages.length - 1].id]) : allIds())} className="press ml-auto rounded-md px-2 py-0.5 font-medium hover:bg-hover hover:text-fg">
              {allOpen ? tr("reader:collapseAll") : tr("reader:expandAll")}
            </button>
          )}
        </div>
      </div>
      <div className="mt-4 space-y-1.5">
        {d.messages.map((m) => (
          <MessageCard key={m.id} m={m} atts={d.attachments.filter((a) => a.messageId === m.id)} me={account?.email ?? ""} expanded={expanded.has(m.id)} onToggle={() => toggle(m.id)} />
        ))}
      </div>
    </div>
  );
}

function BatchButton({ icon: Icon, label, kbd, danger, onClick }: { icon: ComponentType<{ size?: number }>; label: string; kbd?: string; danger?: boolean; onClick: () => void }) {
  return (
    <button onClick={onClick} className={cn("press inline-flex h-9 items-center gap-2 rounded-lg border border-border bg-bg px-3 text-[13px] font-medium hover:bg-hover", danger && "text-danger")}>
      <Icon size={15} />{label}{kbd && <Kbd>{kbd}</Kbd>}
    </button>
  );
}

/** Plusieurs conversations sélectionnées : les actions s'appliquent à tout le lot. */
function MultiSelection({ ids }: { ids: number[] }) {
  const { view, query, threads, folders, moveTo, deleteForever, toggleStar, toggleRead, clearChecked } = useMail(
    useShallow((s) => ({ view: s.view, query: s.query, threads: s.threads, folders: s.folders, moveTo: s.moveTo, deleteForever: s.deleteForever, toggleStar: s.toggleStar, toggleRead: s.toggleRead, clearChecked: s.clearChecked })),
  );
  const picked = threads.filter((t) => ids.includes(t.id));
  const place = actionView({ view, query, threads, folders }, ids);
  const anyUnread = picked.some((t) => t.unreadCount > 0);
  const anyUnstarred = picked.some((t) => !t.isStarred);
  return (
    <main className="anim-fade-in flex h-full min-h-0 min-w-0 flex-col items-center justify-center bg-bg-subtle px-6 text-center" aria-label={tr("reader:multiSelection")}>
      <span className="mb-4 inline-flex h-16 w-16 items-center justify-center rounded-3xl bg-accent-soft text-[24px] font-semibold tnum text-accent">{picked.length}</span>
      <p className="text-[15px] font-semibold">{tr("reader:selected", { count: picked.length })}</p>
      <div className="mt-6 flex max-w-[560px] flex-wrap justify-center gap-2">
        {destinationsFor(place).map((d) => <BatchButton key={d} icon={DESTINATIONS[d].icon} label={DESTINATIONS[d].label} kbd={DESTINATIONS[d].key} onClick={() => moveTo(ids, d)} />)}
        {isPurgeView(place) && <BatchButton icon={Trash2} label={tr("topbar:deleteForever")} kbd="⌫" danger onClick={() => void deleteForever(ids)} />}
        <BatchButton icon={anyUnread ? MailOpen : Mail} label={anyUnread ? tr("reader:markReadMany") : tr("reader:markUnreadMany")} kbd="U" onClick={() => toggleRead(ids)} />
        <BatchButton icon={Star} label={anyUnstarred ? tr("topbar:star") : tr("topbar:unstar")} kbd="S" onClick={() => toggleStar(ids)} />
      </div>
      <button onClick={clearChecked} className="press mt-5 text-[12.5px] text-muted hover:text-fg">{tr("reader:deselect")} <Kbd>{tr("common:keys.escape")}</Kbd></button>
    </main>
  );
}

function EmptyReader() {
  const items: [string, string][] = [["C", tr("sidebar:newMessage")], ["J / K", tr("reader:navigate")], [DESTINATIONS.archive.key, DESTINATIONS.archive.label], [shortcut("mod+k"), tr("reader:palette")]];
  return (
    <div className="anim-fade-in flex h-full flex-col items-center justify-center text-center">
      <span className="mb-5 inline-flex h-16 w-16 items-center justify-center rounded-3xl bg-gradient-to-br from-accent/15 to-[#8b5cf6]/15 text-accent"><Mail size={28} strokeWidth={1.5} /></span>
      <p className="text-[15px] font-semibold">{tr("reader:selectOne")}</p>
      <p className="mt-1 text-[13px] text-muted">{tr("reader:shortcutsHint")} <Kbd>?</Kbd>.</p>
      <div className="mt-6 flex gap-5 text-[12px] text-muted">
        {items.map(([k, label]) => (
          <span key={k} className="flex items-center gap-1.5"><Kbd>{k}</Kbd>{label}</span>
        ))}
      </div>
    </div>
  );
}

export function Reader() {
  const { selectedId, checked, details, threads, accounts } = useMail(
    useShallow((s) => ({ selectedId: s.selectedId, checked: s.checked, details: s.details, threads: s.threads, accounts: s.accounts })),
  );
  const summary = threads.find((t) => t.id === selectedId);
  const d = selectedId === null ? undefined : details[selectedId];
  const account = accounts.find((a) => a.id === (d?.accountId ?? summary?.accountId));
  if (checked.size > 1) return <MultiSelection ids={[...checked]} />;

  return (
    <main className="flex h-full min-h-0 min-w-0 flex-col bg-bg-subtle" aria-label={tr("reader:reading")}>
      <div className="selectable min-h-0 flex-1 overflow-y-auto" key={selectedId ?? "none"}>
        {selectedId === null ? <EmptyReader /> : d ? <ThreadView d={d} account={account} /> : <Skeleton summary={summary} />}
      </div>
    </main>
  );
}

