import * as Dropdown from "@radix-ui/react-dropdown-menu";
import { FileText, Paperclip, Send, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { serverDraftsEnabled, useComposer } from "@/lib/composer";
import type { Template } from "@/lib/api";
import { applyTemplate, htmlToPlain, swapSignature, templateBody } from "@/lib/compose";
import { fileSize } from "@/lib/format";
import { useMail } from "@/lib/store";
import { cn } from "@/lib/utils";
import { btnPrimary, IconButton, Kbd } from "./ui/primitives";
import { RecipientField } from "./RecipientField";
import { RichEditor } from "./RichEditor";
import { shortcut } from "@/lib/platform";

const TITLES = { new: "Nouveau message", reply: "Réponse", replyAll: "Réponse à tous", forward: "Transfert" } as const;

/** Panneau de rédaction flottant (bas droite) : le reste de l'application reste utilisable. */
export function Composer() {
  const draft = useComposer((s) => s.draft);
  const syncState = useComposer((s) => s.syncState);
  const { update, close, discard, send, attach, detach } = useComposer(useShallow((s) => ({ update: s.update, close: s.close, discard: s.discard, send: s.send, attach: s.attach, detach: s.detach })));
  const { backend, accounts, toast } = useMail(useShallow((s) => ({ backend: s.backend, accounts: s.accounts, toast: s.toast })));
  const [showCc, setShowCc] = useState(false);
  const [dragging, setDragging] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const [templates, setTemplates] = useState<Template[]>([]);
  /** Change quand un modèle remplace le corps : l'éditeur relit alors le contenu. */
  const [rev, setRev] = useState(0);

  useEffect(() => setShowCc(false), [draft?.id]);

  if (!draft) return null;
  const fail = (what: string) => (e: unknown) => toast({ message: `${what} impossible — ${String(e)}`, tone: "error" });
  const loadTemplates = () => void backend.listTemplates().then(setTemplates).catch(fail("Chargement des modèles"));
  const insertTemplate = (t: Template) => { update(applyTemplate(draft, t)); setRev((r) => r + 1); };
  const saveAsTemplate = async () => {
    const name = draft.subject.trim() || htmlToPlain(draft.html).slice(0, 40).trim();
    if (!name) return toast({ message: "Écrivez un objet ou un texte avant d'enregistrer un modèle.", tone: "error" });
    const signature = accounts.find((a) => a.id === draft.accountId)?.signature ?? "";
    await backend.saveTemplate({ name, subject: draft.subject, bodyHtml: templateBody(draft, signature) }).then(() => toast({ message: `Modèle « ${name} » enregistré` })).catch(fail("Enregistrement"));
  };
  const removeTemplate = (t: Template) => void backend.deleteTemplate(t.id).then(() => setTemplates((l) => l.filter((x) => x.id !== t.id))).catch(fail("Suppression"));
  const ccOpen = showCc || draft.cc.length > 0 || draft.bcc.length > 0;
  const replying = draft.mode === "reply" || draft.mode === "replyAll";

  return (
    <section
      role="dialog"
      aria-label={TITLES[draft.mode]}
      className="anim-fade-up fixed bottom-4 right-6 z-[60] flex h-[min(640px,calc(100vh-32px))] w-[min(620px,calc(100vw-48px))] flex-col overflow-hidden rounded-2xl border border-border bg-elevated shadow-pop"
      onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); setDragging(true); } }}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(e) => {
        if (!e.dataTransfer.files.length) return;
        e.preventDefault();
        setDragging(false);
        void attach(Array.from(e.dataTransfer.files));
      }}
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); send(); }
        else if (e.key === "Escape") { e.preventDefault(); close(); }
      }}
    >
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-4">
        <h2 className="min-w-0 flex-1 truncate text-[13.5px] font-semibold">{draft.subject || TITLES[draft.mode]}</h2>
        {syncState !== "idle" && serverDraftsEnabled() && (
          <span role="status" className={cn("shrink-0 text-[11.5px]", syncState === "error" ? "text-danger" : "text-faint")} title={syncState === "error" ? "Le brouillon est gardé sur cet ordinateur ; nouvel essai à la prochaine modification." : "Le brouillon est enregistré dans le dossier Brouillons du serveur."}>
            {syncState === "saving" ? "Enregistrement…" : syncState === "saved" ? "Brouillon enregistré" : "Non enregistré sur le serveur"}
          </span>
        )}
        <IconButton label="Fermer (brouillon conservé)" kbd="Échap" onClick={close}><X size={16} /></IconButton>
      </header>

      {accounts.length > 1 && (
        <div className="flex min-h-[38px] items-center gap-2 border-b border-border px-4 text-[13px]">
          <span className="w-6 shrink-0 text-[12px] text-muted">De</span>
          <select
            aria-label="Compte expéditeur"
            value={draft.accountId}
            onChange={(e) => {
              const sig = (id: string) => accounts.find((a) => a.id === id)?.signature ?? "";
              update({ accountId: e.target.value, html: swapSignature(draft.html, sig(draft.accountId), sig(e.target.value)) });
            }}
            className="min-w-0 flex-1 bg-transparent py-1.5 outline-none"
          >
            {accounts.map((a) => <option key={a.id} value={a.id}>{a.displayName ? `${a.displayName} <${a.email}>` : a.email}</option>)}
          </select>
        </div>
      )}

      <div className="relative">
        <RecipientField label="À" value={draft.to} onChange={(to) => update({ to })} backend={backend} accountId={draft.accountId} autoFocus={!replying} />
        {!ccOpen && (
          <button type="button" onClick={() => setShowCc(true)} className="press absolute right-3 top-2 rounded-md px-2 py-0.5 text-[12px] text-muted hover:bg-hover hover:text-fg">Cc / Cci</button>
        )}
      </div>
      {ccOpen && (
        <>
          <RecipientField label="Cc" value={draft.cc} onChange={(cc) => update({ cc })} backend={backend} accountId={draft.accountId} />
          <RecipientField label="Cci" value={draft.bcc} onChange={(bcc) => update({ bcc })} backend={backend} accountId={draft.accountId} />
        </>
      )}

      <input
        aria-label="Objet"
        placeholder="Objet"
        value={draft.subject}
        onChange={(e) => update({ subject: e.target.value })}
        className="h-[42px] shrink-0 border-b border-border bg-transparent px-4 text-[14px] font-medium outline-none placeholder:font-normal placeholder:text-faint"
      />
      {dragging && (
        <div className="pointer-events-none absolute inset-2 z-20 flex items-center justify-center rounded-xl border-2 border-dashed border-accent bg-accent-soft text-[14px] font-medium text-accent">
          Déposez pour joindre
        </div>
      )}
      <RichEditor key={`${draft.id}:${draft.accountId}:${rev}`} html={draft.html} onChange={(html) => update({ html })} focusAtStart={replying} />

      {draft.attachments.length > 0 && (
        <ul className="flex max-h-[84px] shrink-0 flex-wrap gap-1.5 overflow-y-auto border-t border-border px-3 py-2" aria-label="Pièces jointes">
          {draft.attachments.map((a) => (
            <li key={a.id} className="inline-flex max-w-[260px] items-center gap-1.5 rounded-lg bg-hover py-1 pl-2 pr-1 text-[12.5px]">
              <Paperclip size={12} className="shrink-0 text-muted" />
              <span className="truncate" title={a.name}>{a.name}</span>
              <span className="tnum shrink-0 text-muted">{fileSize(a.size)}</span>
              <button type="button" aria-label={`Retirer ${a.name}`} onClick={() => detach(a.id)} className="press rounded p-0.5 text-muted hover:bg-active hover:text-fg"><X size={12} /></button>
            </li>
          ))}
        </ul>
      )}

      <footer className="flex h-14 shrink-0 items-center gap-2 border-t border-border px-3">
        <button type="button" className={`${btnPrimary} h-9 gap-2 px-4`} onClick={() => send()}>
          <Send size={14} /> Envoyer <Kbd className="border-white/30 bg-transparent text-white/80">{shortcut("mod+enter")}</Kbd>
        </button>
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          onChange={(e) => { void attach(Array.from(e.target.files ?? [])); e.target.value = ""; }}
        />
        <IconButton label="Joindre un fichier" onClick={() => fileInput.current?.click()}><Paperclip size={16} /></IconButton>
        <Dropdown.Root onOpenChange={(o) => o && loadTemplates()}>
          <Dropdown.Trigger asChild>
            <IconButton label="Modèles" tooltipSide="top"><FileText size={16} /></IconButton>
          </Dropdown.Trigger>
          <Dropdown.Portal>
            <Dropdown.Content side="top" align="start" sideOffset={8} className="anim-fade-in z-[70] min-w-[220px] max-w-[320px] rounded-xl border border-border bg-elevated p-1 text-[13px] shadow-pop">
              {templates.length === 0 && <div className="px-2.5 py-1.5 text-muted">Aucun modèle enregistré</div>}
              {templates.map((t) => (
                <Dropdown.Item key={t.id} onSelect={() => insertTemplate(t)} className="group/tpl flex cursor-default items-center gap-2 rounded-lg px-2.5 py-1.5 outline-none data-[highlighted]:bg-hover">
                  <span className="min-w-0 flex-1 truncate">{t.name}</span>
                  <button type="button" aria-label={`Supprimer le modèle ${t.name}`} onClick={(e) => { e.stopPropagation(); removeTemplate(t); }} className="press hidden rounded p-0.5 text-muted hover:bg-active hover:text-danger group-data-[highlighted]/tpl:block"><X size={12} /></button>
                </Dropdown.Item>
              ))}
              <Dropdown.Separator className="my-1 h-px bg-border" />
              <Dropdown.Item onSelect={() => void saveAsTemplate()} className="cursor-default rounded-lg px-2.5 py-1.5 outline-none data-[highlighted]:bg-hover">Enregistrer ce message comme modèle</Dropdown.Item>
            </Dropdown.Content>
          </Dropdown.Portal>
        </Dropdown.Root>
        <span className="flex-1" />
        <IconButton label="Supprimer le brouillon" onClick={discard}><Trash2 size={16} /></IconButton>
      </footer>
    </section>
  );
}
