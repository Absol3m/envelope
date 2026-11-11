import { create } from "zustand";
import { attachmentError, draftFrom, forgotAttachment, draftFromServer, emptyDraft, knownAlias, isBlank, readAsBase64, textToHtml, toOutgoing, type ComposeMode, type Draft } from "./compose";
import { pref } from "./pref";
import { useMail } from "./store";

/** Fenêtre d'annulation de l'envoi : rien ne part tant qu'elle n'est pas écoulée (comme l'archivage). */
const UNDO_MS = 5000;
const SAVED_KEY = "envelope.draft";
/** Délai après la dernière modification avant d'enregistrer le brouillon sur le serveur. */
export const SAVE_DELAY_MS = 5000;

export const serverDraftsEnabled = () => pref.get("serverDrafts", "on") === "on";
export const setServerDrafts = (on: boolean) => pref.set("serverDrafts", on ? "on" : "off");

/** Forme écrite dans le stockage local : les pièces jointes (volumineuses) n'y sont pas, seuls leurs noms sont gardés. */
const toSaved = (d: Draft): Draft => ({ ...d, attachments: [], lostAttachments: d.attachments.map((a) => a.name) });

const readSaved = (): Draft | null => {
  try {
    const raw = localStorage.getItem(SAVED_KEY);
    if (!raw) return null;
    // Brouillons d'avant l'éditeur riche : `body` (texte) → `html`, et un identifiant.
    const d = JSON.parse(raw) as Draft & { body?: string };
    return { ...d, id: d.id ?? crypto.randomUUID(), html: d.html ?? textToHtml(d.body ?? ""), attachments: [] };
  } catch { return null; }
};
const writeSaved = (d: Draft | null) => {
  try {
    if (d) localStorage.setItem(SAVED_KEY, JSON.stringify(toSaved(d))); else localStorage.removeItem(SAVED_KEY);
  } catch { /* stockage indisponible : le brouillon reste en mémoire */ }
};

/** État de l'enregistrement du brouillon ouvert sur le serveur. */
export type DraftSync = "idle" | "saving" | "saved" | "error";

interface ComposerState {
  /** Brouillon ouvert dans le panneau (null : panneau fermé). */
  draft: Draft | null;
  syncState: DraftSync;
  /** Rouvre le brouillon d'une conversation du dossier Brouillons. */
  openDraft(threadId: number): Promise<void>;
  /** Dernier brouillon fermé sans être envoyé (un seul emplacement, conservé localement). */
  saved: Draft | null;
  compose(): void;
  respond(mode: Exclude<ComposeMode, "new">): void;
  update(patch: Partial<Draft>): void;
  attach(files: File[]): Promise<void>;
  detach(id: string): void;
  resume(): void;
  /** Ferme le panneau en conservant le brouillon s'il n'est pas vide. */
  close(): void;
  discard(): void;
  /** `force` : envoyer malgré le rappel de pièce jointe oubliée. */
  send(force?: boolean): void;
  /** Annule l'envoi en cours ; faux s'il n'y en a pas. */
  undoSend(): boolean;
}

let pendingSend: { timer: ReturnType<typeof setTimeout>; draft: Draft } | null = null;

// ───────────── Brouillons sur le serveur ─────────────

/** Brouillons présents sur le serveur : identifiant → compte (pour les supprimer au bon endroit). */
const onServer = new Map<string, string>();
/** Dernière version enregistrée de chaque brouillon : rien n'est renvoyé s'il n'a pas changé. */
const lastSig = new Map<string, string>();
/** Brouillons abandonnés pendant qu'un enregistrement était en cours : supprimés dès qu'il se termine. */
const discarded = new Set<string>();
let saveTimer: ReturnType<typeof setTimeout> | undefined;
let saving = false;
let waiting: Draft | null = null;

/** Message à envoyer ou enregistrer : un alias supprimé depuis n'est plus utilisé comme expéditeur. */
const outgoing = (d: Draft) => {
  const account = useMail.getState().accounts.find((a) => a.id === d.accountId);
  return toOutgoing({ ...d, fromEmail: knownAlias(account, d.fromEmail) });
};

const signature = (d: Draft) => JSON.stringify([d.accountId, d.fromEmail, d.to, d.cc, d.bcc, d.subject, d.html, d.attachments.map((a) => a.id)]);

function deleteFromServer(id: string) {
  const account = onServer.get(id);
  onServer.delete(id);
  lastSig.delete(id);
  if (account) void useMail.getState().backend.deleteDraft(account, id).catch(() => {});
}

/** Enregistre `draft` sur le serveur : une seule opération à la fois, la dernière version demandée gagne. */
async function saveToServer(draft: Draft) {
  const backend = useMail.getState().backend;
  const sig = signature(draft);
  if (!serverDraftsEnabled() || backend.demo || isBlank(draft) || lastSig.get(draft.id) === sig) return;
  if (saving) {
    waiting = draft;
    return;
  }
  saving = true;
  useComposer.setState({ syncState: "saving" });
  try {
    await backend.saveDraft(draft.accountId, outgoing(draft));
    onServer.set(draft.id, draft.accountId);
    lastSig.set(draft.id, sig);
    useComposer.setState({ syncState: "saved" });
    if (discarded.delete(draft.id)) deleteFromServer(draft.id);
  } catch {
    useComposer.setState({ syncState: "error" });
  } finally {
    saving = false;
    const next = waiting;
    waiting = null;
    if (next) void saveToServer(next);
  }
}

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
    syncState: "idle",
    saved: readSaved(),

    compose() {
      const { accounts, accountFilter } = useMail.getState();
      const accountId = accountFilter ?? accounts[0]?.id;
      if (!accountId) return toast("Ajoutez d'abord un compte.", "error");
      open(emptyDraft(accountId, accounts.find((a) => a.id === accountId)?.signature));
    },

    respond(mode) {
      const { selectedId, details, accounts } = useMail.getState();
      const thread = selectedId === null ? undefined : details[selectedId];
      const account = accounts.find((a) => a.id === thread?.accountId);
      if (!thread || !account || !thread.messages.length) return toast("Ouvrez une conversation pour répondre.");
      open(draftFrom(mode, thread, account.email, account.signature, account.identities.map((i) => i.email)));
    },

    update(patch) {
      const cur = get().draft;
      // Changer de compte expéditeur : le brouillon quitte le serveur de l'ancien compte (le suivant l'enregistrera ailleurs).
      if (cur && patch.accountId && patch.accountId !== cur.accountId) deleteFromServer(cur.id);
      set((s) => (s.draft ? { draft: { ...s.draft, ...patch } } : s));
    },

    async openDraft(threadId) {
      try {
        const data = await useMail.getState().backend.openDraft(threadId);
        const account = useMail.getState().accounts.find((a) => a.id === data.accountId);
        const draft = { ...draftFromServer(data), fromEmail: knownAlias(account, data.fromEmail) };
        onServer.set(draft.id, draft.accountId);
        lastSig.set(draft.id, signature(draft)); // tel que sur le serveur : pas de nouvel enregistrement tant que rien ne change
        discarded.delete(draft.id);
        open(draft);
        set({ syncState: "idle" });
      } catch (e) {
        toast(`Brouillon impossible à ouvrir — ${String(e)}`, "error", 6000);
      }
    },

    async attach(files) {
      const d = get().draft;
      if (!d || !files.length) return;
      const error = attachmentError(d.attachments, files);
      if (error) return toast(error, "error", 5000);
      try {
        // Lecture fichier par fichier : un seul contenu en transit à la fois, les pastilles apparaissent au fur et à mesure.
        for (const f of files) {
          const added = { id: crypto.randomUUID(), name: f.name, mimeType: f.type || "application/octet-stream", size: f.size, dataBase64: await readAsBase64(f) };
          // Le panneau a pu changer pendant la lecture : on rattache au brouillon d'origine seulement.
          if (get().draft?.id !== d.id) return;
          set((cur) => (cur.draft ? { draft: { ...cur.draft, attachments: [...cur.draft.attachments, added] } } : cur));
        }
      } catch (e) {
        toast(`Pièce jointe impossible — ${String(e)}`, "error");
      }
    },

    detach: (id) => set((s) => (s.draft ? { draft: { ...s.draft, attachments: s.draft.attachments.filter((a) => a.id !== id) } } : s)),

    resume() {
      const saved = get().saved;
      if (!saved) return;
      set({ saved: null });
      writeSaved(null);
      open({ ...saved, lostAttachments: undefined });
      if (saved.lostAttachments?.length) toast(`Pièces jointes à rejoindre : ${saved.lostAttachments.join(", ")}`, "default", 6000);
    },

    close() {
      const d = get().draft;
      if (!d) return;
      if (isBlank(d)) {
        deleteFromServer(d.id); // tout a été effacé : plus rien à garder sur le serveur
        return set({ draft: null });
      }
      clearTimeout(saveTimer);
      void saveToServer(d); // version finale sur le serveur ; la copie locale reste le tampon hors ligne
      writeSaved(d);
      set({ draft: null, saved: d });
      useMail.getState().toast({ message: "Brouillon conservé", actionLabel: "Reprendre", onAction: () => get().resume(), duration: 4000 });
    },

    discard() {
      const d = get().draft;
      set({ draft: null });
      if (!d) return;
      if (saving) discarded.add(d.id); // un enregistrement est en cours : on supprimera à sa fin
      deleteFromServer(d.id);
    },

    send(force = false) {
      const draft = get().draft;
      if (!draft) return;
      if (!draft.to.length) return toast("Ajoutez au moins un destinataire.", "error");
      if (!force && forgotAttachment(draft)) {
        return useMail.getState().toast({ message: "Vous mentionnez une pièce jointe, mais aucune n'est jointe.", tone: "error", actionLabel: "Envoyer quand même", onAction: () => get().send(true), duration: 8000 });
      }
      set({ draft: null });
      pendingSend = {
        draft,
        timer: setTimeout(async () => {
          pendingSend = null;
          try {
            const report = await useMail.getState().backend.sendEmail(draft.accountId, outgoing(draft));
            onServer.delete(draft.id); // le serveur retire le brouillon de lui-même après un envoi réussi
            lastSig.delete(draft.id);
            const failure = typeof report.sentCopy === "object" ? report.sentCopy.failed : null;
            if (failure === null) toast("Message envoyé");
            else toast(`Message envoyé, mais la copie dans « Envoyés » a échoué — ${failure}`, "error", 8000);
          } catch (e) {
            open(draft); // rien n'est perdu : le message revient dans le panneau
            toast(`Envoi impossible — ${String(e)}`, "error", 7000);
          }
        }, UNDO_MS),
      };
      useMail.getState().toast({ message: "Envoi du message…", actionLabel: "Annuler", onAction: () => get().undoSend(), duration: UNDO_MS, countdown: true });
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

// Chaque modification du brouillon ouvert relance un délai ; il part sur le serveur quand on s'arrête d'écrire.
useComposer.subscribe((state, prev) => {
  if (!state.draft) return clearTimeout(saveTimer);
  if (state.draft === prev.draft) return; // changement d'état d'enregistrement seulement
  clearTimeout(saveTimer);
  const id = state.draft.id;
  saveTimer = setTimeout(() => {
    const cur = useComposer.getState().draft;
    if (cur?.id === id) void saveToServer(cur);
  }, SAVE_DELAY_MS);
});
