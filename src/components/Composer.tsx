import { Send, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useComposer } from "@/lib/composer";
import { useMail } from "@/lib/store";
import { btnPrimary, IconButton, Kbd } from "./ui/primitives";
import { RecipientField } from "./RecipientField";
import { RichEditor } from "./RichEditor";

const TITLES = { new: "Nouveau message", reply: "Réponse", replyAll: "Réponse à tous", forward: "Transfert" } as const;

/** Panneau de rédaction flottant (bas droite) : le reste de l'application reste utilisable. */
export function Composer() {
  const draft = useComposer((s) => s.draft);
  const { update, close, discard, send } = useComposer(useShallow((s) => ({ update: s.update, close: s.close, discard: s.discard, send: s.send })));
  const { backend, accounts } = useMail(useShallow((s) => ({ backend: s.backend, accounts: s.accounts })));
  const [showCc, setShowCc] = useState(false);

  useEffect(() => setShowCc(false), [draft?.id]);

  if (!draft) return null;
  const replying = draft.mode === "reply" || draft.mode === "replyAll";
  const submit = () => {
    window.dispatchEvent(new Event("envelope:commit-recipients")); // saisie en cours → pastille
    send();
  };

  return (
    <section
      role="dialog"
      aria-label={TITLES[draft.mode]}
      className="anim-fade-up fixed bottom-4 right-6 z-[60] flex h-[min(640px,calc(100vh-32px))] w-[min(620px,calc(100vw-48px))] flex-col overflow-hidden rounded-2xl border border-border bg-elevated shadow-pop"
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); submit(); }
        else if (e.key === "Escape") { e.preventDefault(); close(); }
      }}
    >
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-4">
        <h2 className="min-w-0 flex-1 truncate text-[13.5px] font-semibold">{draft.subject || TITLES[draft.mode]}</h2>
        <IconButton label="Fermer (brouillon conservé)" kbd="Échap" onClick={close}><X size={16} /></IconButton>
      </header>

      {accounts.length > 1 && (
        <div className="flex min-h-[38px] items-center gap-2 border-b border-border px-4 text-[13px]">
          <span className="w-6 shrink-0 text-[12px] text-muted">De</span>
          <select
            aria-label="Compte expéditeur"
            value={draft.accountId}
            onChange={(e) => update({ accountId: e.target.value })}
            className="min-w-0 flex-1 bg-transparent py-1.5 outline-none"
          >
            {accounts.map((a) => <option key={a.id} value={a.id}>{a.displayName ? `${a.displayName} <${a.email}>` : a.email}</option>)}
          </select>
        </div>
      )}

      <div className="relative">
        <RecipientField label="À" value={draft.to} onChange={(to) => update({ to })} backend={backend} accountId={draft.accountId} autoFocus={!replying} />
        {!(showCc || draft.cc.length || draft.bcc.length) && (
          <button type="button" onClick={() => setShowCc(true)} className="press absolute right-3 top-2 rounded-md px-2 py-0.5 text-[12px] text-muted hover:bg-hover hover:text-fg">Cc / Cci</button>
        )}
      </div>
      {(showCc || draft.cc.length > 0 || draft.bcc.length > 0) && (
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
      <RichEditor key={draft.id} html={draft.html} onChange={(html) => update({ html })} focusAtStart={replying} />

      <footer className="flex h-14 shrink-0 items-center gap-2 border-t border-border px-3">
        <button type="button" className={`${btnPrimary} h-9 gap-2 px-4`} onClick={submit}>
          <Send size={14} /> Envoyer <Kbd className="border-white/30 bg-transparent text-white/80">⌘↵</Kbd>
        </button>
        <span className="flex-1" />
        <IconButton label="Supprimer le brouillon" onClick={discard}><Trash2 size={16} /></IconButton>
      </footer>
    </section>
  );
}
