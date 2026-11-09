import { create } from "zustand";
import { applyTheme, loadTheme, type ThemePref } from "./theme";
import { demoBackend } from "./mock";
import { isTauri, tauriBackend, type Account, type Backend, type FolderRow, type ThreadDetail, type ThreadQuery, type ThreadSummary } from "./api";
import { categorize, type Category } from "./categories";
import { isInboxView, type View } from "./views";

export type { View };

export type SettingsTab = "accounts" | "agent" | "appearance" | "shortcuts";
export type Density = "comfortable" | "compact";

export interface Toast {
  id: number;
  message: string;
  tone: "default" | "error";
  actionLabel?: string;
  onAction?: () => void;
  duration: number;
}

const PAGE = 100;
const UNDO_MS = 5000;

const pref = {
  get<T extends string>(key: string, fallback: T): T {
    try {
      return (localStorage.getItem(`envelope.${key}`) as T | null) ?? fallback;
    } catch {
      return fallback;
    }
  },
  set(key: string, value: string) {
    try {
      localStorage.setItem(`envelope.${key}`, value);
    } catch {
      /* non mémorisé */
    }
  },
};

interface State {
  backend: Backend;
  ready: boolean;
  needsOnboarding: boolean;
  accounts: Account[];
  folders: FolderRow[];
  accountFilter: string | null;
  view: View;
  category: Category;
  query: string;
  threads: ThreadSummary[];
  threadsLoading: boolean;
  hasMore: boolean;
  selectedId: number | null;
  /** Conversations archivées en attente de validation (fenêtre d'annulation) : masquées de la liste. */
  hiddenIds: ReadonlySet<number>;
  details: Record<number, ThreadDetail>;
  toasts: Toast[];
  paletteOpen: boolean;
  settingsTab: SettingsTab | null;
  helpOpen: boolean;
  theme: ThemePref;
  density: Density;
  previews: boolean;

  init(): Promise<void>;
  enterDemo(): Promise<void>;
  reload(opts?: { silent?: boolean }): Promise<void>;
  refreshAccounts(): Promise<void>;
  loadMore(): Promise<void>;
  setView(v: View): void;
  setAccount(id: string | null): void;
  setCategory(c: Category): void;
  setQuery(q: string): void;
  select(id: number | null): void;
  move(delta: 1 | -1): void;
  toggleRead(id: number): void;
  toggleStar(id: number): void;
  archive(id: number): void;
  undo(): void;
  toast(t: Partial<Toast> & { message: string }): void;
  dismissToast(id: number): void;
  setPalette(open: boolean): void;
  openSettings(tab: SettingsTab | null): void;
  setHelp(open: boolean): void;
  setTheme(t: ThemePref): void;
  setDensity(d: Density): void;
  setPreviews(p: boolean): void;
}

let toastSeq = 0;
let searchTimer: ReturnType<typeof setTimeout> | undefined;
let markReadTimer: ReturnType<typeof setTimeout> | undefined;
let loadSeq = 0; // une réponse tardive d'une ancienne requête ne doit pas écraser la liste actuelle
/** Archivages dans leur fenêtre d'annulation : id de conversation → minuterie (le dernier ajouté est annulé par Z). */
const pending = new Map<number, ReturnType<typeof setTimeout>>();

/** Navigation remise à zéro : état initial et entrée en démo partagent ces valeurs. */
const NAV_RESET = { accountFilter: null, view: { kind: "role", role: "inbox" } as View, category: "all" as Category, query: "", selectedId: null, hiddenIds: new Set<number>() as ReadonlySet<number> };

function buildQuery(s: Pick<State, "view" | "accountFilter" | "query">): ThreadQuery {
  const q: ThreadQuery = { limit: PAGE };
  if (s.accountFilter) q.accountId = s.accountFilter;
  if (s.query.trim()) q.query = s.query.trim();
  const v = s.view;
  switch (v.kind) {
    case "role": q.role = v.role; break;
    case "starred": q.starredOnly = true; break;
    case "unread": q.role = "inbox"; q.unreadOnly = true; break;
    case "attachments": q.withAttachments = true; break;
    case "folder": q.folderId = v.folderId; break;
  }
  return q;
}

/** Conversations affichées : sans celles en cours d'archivage, filtrées par catégorie Smart Inbox. */
export function visibleThreads(s: Pick<State, "threads" | "hiddenIds" | "category" | "view" | "query">): ThreadSummary[] {
  const smart = isInboxView(s.view) && !s.query.trim() && s.category !== "all";
  return s.threads.filter((t) => !s.hiddenIds.has(t.id) && (!smart || categorize(t) === s.category));
}

export const useMail = create<State>((set, get) => {
  const patchThread = (id: number, patch: Partial<ThreadSummary>) =>
    set((s) => ({ threads: s.threads.map((t) => (t.id === id ? { ...t, ...patch } : t)) }));
  const patchDetail = (id: number, fn: (d: ThreadDetail) => ThreadDetail) =>
    set((s) => (s.details[id] ? { details: { ...s.details, [id]: fn(s.details[id]) } } : s));
  const loadFolders = async () => {
    try { set({ folders: await get().backend.listFolders() }); } catch { /* compteurs : non bloquant */ }
  };
  /** Après une action : un seul rafraîchissement des compteurs, même si les actions s'enchaînent. */
  let foldersTimer: ReturnType<typeof setTimeout> | undefined;
  const scheduleFolders = () => {
    clearTimeout(foldersTimer);
    foldersTimer = setTimeout(() => void loadFolders(), 500);
  };
  const setHidden = (id: number, hidden: boolean) =>
    set((cur) => {
      const h = new Set(cur.hiddenIds);
      if (hidden) h.add(id); else h.delete(id);
      return { hiddenIds: h };
    });
  const fail = (message: string, e: unknown) => get().toast({ message: `${message} — ${String(e)}`, tone: "error", duration: 6000 });

  async function fetchThreads(reset: boolean, silent = false) {
    const seq = ++loadSeq;
    const s = get();
    if (!silent) set({ threadsLoading: true });
    try {
      const rows = await s.backend.listThreads({ ...buildQuery(s), offset: reset ? 0 : s.threads.length });
      if (seq !== loadSeq) return;
      set((cur) => ({ threads: reset ? rows : [...cur.threads, ...rows], hasMore: rows.length === PAGE, threadsLoading: false }));
    } catch (e) {
      if (seq !== loadSeq) return;
      set({ threadsLoading: false });
      fail("Chargement impossible", e);
    }
  }

  const selectFirst = () => get().select(visibleThreads(get())[0]?.id ?? null);

  /** Après un changement de vue/filtre : recharge la liste et reprend la sélection sur le premier élément. */
  const refetchAndSelect = async () => {
    await fetchThreads(true);
    selectFirst();
  };

  /** Affichage optimiste : l'état est déjà modifié ; si le serveur refuse, `revert` le rétablit et l'erreur s'affiche. */
  const optimistic = (call: Promise<unknown>, revert: () => void, label = "Action impossible", onOk?: () => void) =>
    call.then(() => { onOk?.(); scheduleFolders(); }, (e) => { revert(); fail(label, e); });

  return {
    backend: demoBackend,
    ready: false,
    needsOnboarding: false,
    accounts: [],
    folders: [],
    ...NAV_RESET,
    threads: [],
    threadsLoading: true,
    hasMore: false,
    details: {},
    toasts: [],
    paletteOpen: false,
    settingsTab: null,
    helpOpen: false,
    theme: loadTheme(),
    density: pref.get<Density>("density", "comfortable"),
    previews: pref.get("previews", "on") === "on",

    async init() {
      if (!isTauri()) return get().enterDemo(); // aperçu navigateur : boîte de démonstration
      set({ backend: tauriBackend });
      try {
        const accounts = await tauriBackend.listAccounts();
        set({ accounts, needsOnboarding: accounts.length === 0, ready: true });
        if (accounts.length) {
          await loadFolders();
          await refetchAndSelect();
        } else {
          set({ threadsLoading: false });
        }
      } catch (e) {
        set({ ready: true, threadsLoading: false });
        fail("Démarrage impossible", e);
      }
    },

    async enterDemo() {
      set({ backend: demoBackend, needsOnboarding: false, threads: [], details: {}, ...NAV_RESET });
      const accounts = await demoBackend.listAccounts();
      set({ accounts, ready: true });
      await loadFolders();
      await refetchAndSelect();
    },

    async reload(opts) {
      const b = get().backend;
      try {
        const accounts = await b.listAccounts();
        set({ accounts, needsOnboarding: !b.demo && accounts.length === 0 });
      } catch { /* conservé */ }
      await loadFolders();
      await fetchThreads(true, opts?.silent);
    },

    async refreshAccounts() {
      try { set({ accounts: await get().backend.listAccounts() }); } catch { /* conservé */ }
    },

    async loadMore() {
      const s = get();
      if (s.hasMore && !s.threadsLoading) await fetchThreads(false, true);
    },

    setView(view) {
      set({ view, category: "all", query: "", selectedId: null });
      void refetchAndSelect();
    },
    setAccount(accountFilter) {
      set({ accountFilter });
      void refetchAndSelect();
    },
    setCategory(category) {
      set({ category });
      selectFirst();
    },
    setQuery(query) {
      set({ query, threadsLoading: true });
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => void refetchAndSelect(), 180);
    },

    select(id) {
      clearTimeout(markReadTimer);
      set({ selectedId: id });
      if (id === null) return;
      const s = get();
      if (!s.details[id]) {
        s.backend.getThread(id).then(
          (d) => set((cur) => ({ details: { ...cur.details, [id]: d } })),
          (e) => fail("Conversation illisible", e),
        );
      }
      // Lue après un court instant d'affichage : parcourir la liste (J/K) ne marque rien comme lu.
      markReadTimer = setTimeout(() => {
        const t = get().threads.find((x) => x.id === id);
        if (t && t.unreadCount > 0 && get().selectedId === id) get().toggleRead(id);
      }, 900);
    },

    move(delta) {
      const s = get();
      const list = visibleThreads(s);
      if (list.length === 0) return;
      const i = list.findIndex((t) => t.id === s.selectedId);
      const next = list[Math.min(list.length - 1, Math.max(0, i < 0 ? 0 : i + delta))];
      if (next) s.select(next.id);
    },

    toggleRead(id) {
      const t = get().threads.find((x) => x.id === id);
      if (!t) return;
      const read = t.unreadCount > 0; // bascule : non lu → lu, lu → non lu
      const prev = t.unreadCount;
      patchThread(id, { unreadCount: read ? 0 : 1 });
      patchDetail(id, (d) => ({ ...d, messages: d.messages.map((m, i, a) => ({ ...m, isRead: read ? true : i < a.length - 1 })) }));
      void optimistic(get().backend.setRead(id, read), () => patchThread(id, { unreadCount: prev }));
    },

    toggleStar(id) {
      const t = get().threads.find((x) => x.id === id);
      if (!t) return;
      const starred = !t.isStarred;
      patchThread(id, { isStarred: starred });
      patchDetail(id, (d) => ({ ...d, isStarred: starred }));
      void optimistic(get().backend.setStarred(id, starred), () => patchThread(id, { isStarred: !starred }), "Action impossible", () => {
        // Dans la vue « Suivis », retirer l'étoile fait sortir la conversation de la liste.
        if (!starred && get().view.kind === "starred") get().select(visibleThreads(get()).find((x) => x.id !== id && x.isStarred)?.id ?? null);
      });
    },

    archive(id) {
      const s = get();
      const thread = s.threads.find((t) => t.id === id);
      if (!thread) return;
      if (s.view.kind === "role" && s.view.role === "archive") return s.toast({ message: "Cette conversation est déjà archivée." });

      const folderId = s.view.kind === "folder" ? s.view.folderId : s.folders.find((f) => f.accountId === thread.accountId && f.role === "inbox")?.id;
      if (folderId === undefined) return s.toast({ message: "Dossier de réception introuvable.", tone: "error" });

      // Sélection suivante avant masquage : l'écran ne se vide jamais.
      const list = visibleThreads(s);
      const i = list.findIndex((t) => t.id === id);
      const next = list[i + 1] ?? list[i - 1];
      setHidden(id, true);
      if (s.selectedId === id) get().select(next ? next.id : null);

      // Rien n'est envoyé au serveur tant que l'annulation est possible.
      pending.set(id, setTimeout(() => {
        pending.delete(id);
        void optimistic(get().backend.archive(id, folderId), () => setHidden(id, false), "Archivage impossible", () =>
          set((cur) => ({ threads: cur.threads.filter((t) => t.id !== id) })),
        );
      }, UNDO_MS));
      get().toast({ message: "Conversation archivée", actionLabel: "Annuler", onAction: () => get().undo(), duration: UNDO_MS });
    },

    undo() {
      const last = [...pending].pop();
      if (!last) return;
      const [id, timer] = last;
      clearTimeout(timer);
      pending.delete(id);
      setHidden(id, false);
      get().select(id);
      get().toast({ message: "Archivage annulé", duration: 2200 });
    },

    toast(t) {
      const id = ++toastSeq;
      const toast: Toast = { id, tone: "default", duration: 3200, ...t };
      // Un seul toast à la fois : le dernier remplace le précédent (pas d'empilement bruyant).
      set({ toasts: [toast] });
      setTimeout(() => get().dismissToast(id), toast.duration);
    },
    dismissToast(id) {
      set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
    },

    setPalette: (paletteOpen) => set({ paletteOpen }),
    openSettings: (settingsTab) => set({ settingsTab }),
    setHelp: (helpOpen) => set({ helpOpen }),
    setTheme(theme) {
      applyTheme(theme);
      set({ theme });
    },
    setDensity(density) {
      pref.set("density", density);
      set({ density });
    },
    setPreviews(previews) {
      pref.set("previews", previews ? "on" : "off");
      set({ previews });
    },
  };
});
