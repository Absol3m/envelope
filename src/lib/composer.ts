import { create } from "zustand";
import { draftFrom, emptyDraft, isBlank, textToHtml, toOutgoing, type ComposeMode, type Draft } from "./compose";
import { useMail } from "./store";

/** Fenêtre d'annulation de l'envoi : rien ne part tant qu'elle n'est pas écoulée (comme l'archivage). */
const UNDO_MS = 5000;
const SAVED_KEY = "envelope.draft";

const readSaved = (): Draft | null => {
  try {
    const raw = localStorage.getItem(SAVED_KEY);
    if (!raw) return null;
    // Brouillons d'avant l'éditeur riche : `body` (texte) → `html`, et un identifiant.
    const d = JSON.parse(raw) as Draft & { body?: string };
    return { ...d, id: d.id ?? crypto.randomUUID(), html: d.html ?? textToHtml(d.body ?? "") };
  } catch { return null; }
};
const writeSaved = (d: Draft | null) => {
  try {
    if (d) localStorage.setItem(SAVED_KEY, JSON.stringify(d)); else localStorage.removeItem(SAVED_KEY);
  } catch { /* stockage indisponible : le brouillon reste en mémoire */ }
};

interface ComposerState {
  /** Brouillon ouvert dans le panneau (null : panneau fermé). */
  draft: Draft | null;
  /** Dernier brouillon fermé sans être envoyé (un seul emplacement, conservé localement). */
  saved: Draft | null;
  compose(): void;
  respond(mode: Exclude<ComposeMode, "new">): void;
  update(patch: Partial<Draft>): void;
  resume(): void;
  /** Ferme le panneau en conservant le brouillon s'il n'est pas vide. */
  close(): void;
  discard(): void;
  send(): void;
  /** Annule l'envoi en cours ; faux s'il n'y en a pas. */
  undoSend(): boolean;
}

let pendingSend: { timer: ReturnType<typeof setTimeout>; draft: Draft } | null = null;

export const useComposer = create<ComposerState>((set, get) => {
  const toast = (message: string, tone: "default" | "error" = "default", duration?: number) => useMail.getState().toast({ message, tone, duration });
  /** Ouvre `draft` ; un brouillon déjà en cours n'est jamais écrasé, il est mis de côté. */
  const open = (draft: Draft) => {
    const cur = get().draft;
    const saved = cur && !isBlank(cur) ? cur : get().saved;
    writeSaved(saved);
    set({ draft, saved });
  };

  return {
    draft: null,
    saved: readSaved(),

    compose() {
      const { accounts, accountFilter } = useMail.getState();
      const accountId = accountFilter ?? accounts[0]?.id;
      if (!accountId) return toast("Ajoutez d'abord un compte.", "error");
      open(emptyDraft(accountId));
    },

    respond(mode) {
      const { selectedId, details, accounts } = useMail.getState();
      const thread = selectedId === null ? undefined : details[selectedId];
      const own = accounts.find((a) => a.id === thread?.accountId)?.email;
      if (!thread || !own || !thread.messages.length) return toast("Ouvrez une conversation pour répondre.");
      open(draftFrom(mode, thread, own));
    },

    update: (patch) => set((s) => (s.draft ? { draft: { ...s.draft, ...patch } } : s)),

    resume() {
      const saved = get().saved;
      if (!saved) return;
      set({ saved: null });
      writeSaved(null);
      open(saved);
    },

    close() {
      const d = get().draft;
      if (!d) return;
      if (isBlank(d)) return set({ draft: null });
      writeSaved(d);
      set({ draft: null, saved: d });
      useMail.getState().toast({ message: "Brouillon conservé", actionLabel: "Reprendre", onAction: () => get().resume(), duration: 4000 });
    },

    discard() {
      set({ draft: null });
    },

    send() {
      const draft = get().draft;
      if (!draft) return;
      if (!draft.to.length) return toast("Ajoutez au moins un destinataire.", "error");
      set({ draft: null });
      pendingSend = {
        draft,
        timer: setTimeout(async () => {
          pendingSend = null;
          try {
            const report = await useMail.getState().backend.sendEmail(draft.accountId, toOutgoing(draft));
            toast(report.savedToSent === false ? `Message envoyé, mais la copie dans « Envoyés » a échoué — ${report.sentError ?? "cause inconnue"}` : "Message envoyé", report.savedToSent === false ? "error" : "default", report.savedToSent === false ? 8000 : undefined);
          } catch (e) {
            open(draft); // rien n'est perdu : le message revient dans le panneau
            toast(`Envoi impossible — ${String(e)}`, "error", 7000);
          }
        }, UNDO_MS),
      };
      useMail.getState().toast({ message: "Envoi du message…", actionLabel: "Annuler", onAction: () => get().undoSend(), duration: UNDO_MS });
    },

    undoSend() {
      if (!pendingSend) return false;
      clearTimeout(pendingSend.timer);
      const { draft } = pendingSend;
      pendingSend = null;
      open(draft);
      toast("Envoi annulé", "default", 2200);
      return true;
    },
  };
});
