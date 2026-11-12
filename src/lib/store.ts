import { create } from "zustand";
import { parseColors, parseTheme, renderAppearance, type ColorsPref, type ThemePref } from "./theme";
import { applyFont, savedFont, type FontPref } from "./font";
import { pref } from "./pref";
import { demoBackend } from "./mock";
import { isTauri, tauriBackend, type Account, type Backend, type FolderRow, type LockStatus, type ReminderKind, type ScheduledSend, type ThreadDetail, type ThreadQuery, type ThreadSummary } from "./api";
import { categorize, type Category } from "./categories";
import { DESTINATIONS, isEmptiableView, isInboxView, isPurgeView, LOCAL_ACCOUNT, placeOf, type Destination, type View } from "./views";

export type { View };

export type SettingsTab = "accounts" | "rules" | "security" | "agent" | "appearance" | "shortcuts";
export type Density = "comfortable" | "compact";

export interface Toast {
  id: number;
  message: string;
  tone: "default" | "error";
  actionLabel?: string;
  onAction?: () => void;
  duration: number;
  /** Affiche la barre de compte à rebours : réservé aux actions réellement différées jusqu'à la fin du délai (envoi). */
  countdown?: boolean;
}

const PAGE = 100;

/** Une ou plusieurs conversations : les actions s'appliquent à l'ensemble. */
type Ids = number | readonly number[];
const toIds = (ids: Ids): readonly number[] => (typeof ids === "number" ? [ids] : ids);
const NO_IDS: ReadonlySet<number> = new Set();

/** Lance les opérations d'un lot ; sépare celles qui ont abouti de celles qui ont échoué (première raison conservée). */
async function settleBatch<T>(items: readonly T[], run: (item: T) => Promise<unknown>) {
  const results = await Promise.allSettled(items.map(run));
  const ok = new Set(results.flatMap((r, i) => (r.status === "fulfilled" ? [i] : [])));
  const failure = results.find((r): r is PromiseRejectedResult => r.status === "rejected");
  return { done: items.filter((_, i) => ok.has(i)), failed: items.filter((_, i) => !ok.has(i)), reason: failure?.reason };
}


interface State {
  backend: Backend;
  ready: boolean;
  needsOnboarding: boolean;
  importOpen: boolean;
  /** Envois programmés en file (liste « Programmés »). */
  scheduled: ScheduledSend[];
  /** Verrouillage de l'application (null tant qu'il n'est pas connu). Verrouillé : seul l'écran de déverrouillage s'affiche. */
  lock: LockStatus | null;
  scheduledOpen: boolean;
  /** Historique encore à rapatrier du serveur, par compte (absent ou 0 : complet). */
  backfill: Readonly<Record<string, number>>;
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
  /** Sélection multiple (Maj/Cmd/Ctrl+clic) : prend le pas sur `selectedId` dès qu'elle compte 2 conversations ou plus. */
  checked: ReadonlySet<number>;
  /** Point de départ des sélections par plage (Maj+clic). */
  anchorId: number | null;
  /** Conversations archivées en attente de validation (fenêtre d'annulation) : masquées de la liste. */
  hiddenIds: ReadonlySet<number>;
  details: Record<number, ThreadDetail>;
  toasts: Toast[];
  paletteOpen: boolean;
  settingsTab: SettingsTab | null;
  helpOpen: boolean;
  theme: ThemePref;
  colors: ColorsPref;
  density: Density;
  font: FontPref;
  previews: boolean;

  init(): Promise<void>;
  enterDemo(): Promise<void>;
  reload(opts?: { silent?: boolean }): Promise<void>;
  refreshAccounts(): Promise<void>;
  setBackfill(accountId: string, remaining: number): void;
  setImportOpen(open: boolean): void;
  setScheduledOpen(open: boolean): void;
  /** Verrouille tout de suite : le moteur refuse alors toute commande, et les données affichées sont effacées. */
  lockNow(): Promise<void>;
  /** Après un déverrouillage réussi : recharge la boîte. */
  unlocked(): Promise<void>;
  setLockStatus(lock: LockStatus): void;
  refreshScheduled(): Promise<void>;
  loadMore(): Promise<void>;
  setView(v: View): void;
  setAccount(id: string | null): void;
  setCategory(c: Category): void;
  setQuery(q: string): void;
  select(id: number | null): void;
  toggleCheck(id: number): void;
  rangeCheck(id: number): void;
  checkAll(): void;
  clearChecked(): void;
  move(delta: 1 | -1): void;
  toggleRead(ids: Ids): void;
  toggleStar(ids: Ids): void;
  /** Pose un rappel (report ou relance) ; `at` en millisecondes. */
  remind(id: number, kind: ReminderKind, at: number): void;
  clearReminder(id: number): void;
  /** Archive, corbeille ou spam, avec fenêtre d'annulation. */
  /** `targetFolderId` : dossier local visé quand `to` vaut « local » (par défaut « Archives »). */
  moveTo(ids: Ids, to: Destination, targetFolderId?: number): void;
  /** Suppression définitive depuis la corbeille ou le spam, après confirmation. */
  deleteForever(ids: Ids): Promise<void>;
  /** Vide la corbeille ou le spam affiché (tous les comptes de la vue), après confirmation. */
  emptyView(): Promise<void>;
  toast(t: Partial<Toast> & { message: string }): void;
  dismissToast(id: number): void;
  setPalette(open: boolean): void;
  openSettings(tab: SettingsTab | null): void;
  setHelp(open: boolean): void;
  setTheme(t: ThemePref): void;
  setColors(c: ColorsPref): void;
  setDensity(d: Density): void;
  setFont(f: FontPref): void;
  setPreviews(p: boolean): void;
}

let toastSeq = 0;
let searchTimer: ReturnType<typeof setTimeout> | undefined;
let markReadTimer: ReturnType<typeof setTimeout> | undefined;
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let loadSeq = 0; // une réponse tardive d'une ancienne requête ne doit pas écraser la liste actuelle
/** Dossier d'où part une conversation : celui de la vue affichée, à défaut (vues transverses) la boîte de réception. */
function sourceFolder(s: Pick<State, "view" | "folders" | "query">, thread: ThreadSummary) {
  // Résultats de recherche : la conversation vient du dossier où elle se trouve, pas de la vue affichée.
  if (s.query.trim() && thread.folderId !== null) return s.folders.find((f) => f.id === thread.folderId);
  const v = s.view;
  const role = v.kind === "role" ? v.role : "inbox";
  return s.folders.find((f) => (v.kind === "folder" ? f.id === v.folderId : f.accountId === thread.accountId && f.role === role));
}


/** Vue qui détermine les actions possibles sur ces conversations : en recherche, le dossier de la première. */
export function actionView(s: Pick<State, "view" | "query" | "threads" | "folders">, ids: readonly number[]): View {
  if (!s.query.trim()) return s.view;
  const t = s.threads.find((x) => x.id === ids[0]);
  return (t && placeOf(t, s.folders)) ?? s.view;
}

/** Navigation remise à zéro : état initial et entrée en démo partagent ces valeurs. */
const NAV_RESET = { accountFilter: null, view: { kind: "role", role: "inbox" } as View, category: "all" as Category, query: "", selectedId: null, checked: NO_IDS, anchorId: null as number | null, hiddenIds: new Set<number>() as ReadonlySet<number> };

function buildQuery(s: Pick<State, "view" | "accountFilter" | "query">): ThreadQuery {
  const q: ThreadQuery = { limit: PAGE };
  if (s.accountFilter) q.accountId = s.accountFilter;
  // Une recherche couvre tous les dossiers (hors corbeille et spam, sauf `dans:`), quelle que soit la vue affichée.
  if (s.query.trim()) return { ...q, query: s.query.trim() };
  const v = s.view;
  switch (v.kind) {
    case "role": q.role = v.role; break;
    case "starred": q.starredOnly = true; break;
    case "unread": q.role = "inbox"; q.unreadOnly = true; break;
    case "attachments": q.withAttachments = true; break;
    case "reminders": q.remindersOnly = true; break;
    case "folder": q.folderId = v.folderId; break;
  }
  return q;
}

/** Conversations affichées : sans celles en cours d'archivage, filtrées par catégorie Smart Inbox. */
export function visibleThreads(s: Pick<State, "threads" | "hiddenIds" | "category" | "view" | "query">): ThreadSummary[] {
  const smart = isInboxView(s.view) && !s.query.trim() && s.category !== "all";
  return s.threads.filter((t) => !s.hiddenIds.has(t.id) && (!smart || categorize(t) === s.category));
}

/** Conversations visées par une action : la sélection multiple, à défaut la conversation ouverte. */
export function selectionIds(s: Pick<State, "checked" | "selectedId">): number[] {
  return s.checked.size > 1 ? [...s.checked] : s.selectedId !== null ? [s.selectedId] : [];
}

/** Conversations emportées par un glisser depuis cette ligne : toute la sélection si elle en fait partie. */
export function dragIds(s: Pick<State, "checked">, id: number): number[] {
  return s.checked.size > 1 && s.checked.has(id) ? [...s.checked] : [id];
}

export const useMail = create<State>((set, get) => {
  const patchThread = (id: number, patch: Partial<ThreadSummary>) =>
    set((s) => ({ threads: s.threads.map((t) => (t.id === id ? { ...t, ...patch } : t)) }));
  const patchDetail = (id: number, fn: (d: ThreadDetail) => ThreadDetail) =>
    set((s) => (s.details[id] ? { details: { ...s.details, [id]: fn(s.details[id]) } } : s));
  /** Conversations de la liste correspondant à ces identifiants (une seule passe, pas de `includes` en boucle). */
  const threadsOf = (ids: Ids) => {
    const wanted = new Set(toIds(ids));
    return get().threads.filter((t) => wanted.has(t.id));
  };
  let badge = -1;
  const loadFolders = async () => {
    try {
      const folders = await get().backend.listFolders();
      set({ folders });
      // Pastille sur l'icône : non-lus de toutes les boîtes de réception (appel natif seulement si le total change).
      const unread = folders.reduce((n, f) => n + (f.role === "inbox" ? f.unreadCount : 0), 0);
      if (unread !== badge) {
        badge = unread;
        void get().backend.setBadge(unread).catch(() => { badge = -1; });
      }
    } catch { /* compteurs : non bloquant */ }
  };
  /** Après une action : un seul rafraîchissement des compteurs, même si les actions s'enchaînent. */
  let foldersTimer: ReturnType<typeof setTimeout> | undefined;
  const scheduleFolders = () => {
    clearTimeout(foldersTimer);
    foldersTimer = setTimeout(() => void loadFolders(), 500);
  };
  const setHidden = (ids: number | readonly number[], hidden: boolean) =>
    set((cur) => {
      const h = new Set(cur.hiddenIds);
      for (const id of toIds(ids)) { if (hidden) h.add(id); else h.delete(id); }
      return { hiddenIds: h };
    });
  /** Retire de la liste des conversations déplacées ou supprimées côté serveur, puis rafraîchit les compteurs. */
  const dropThreads = (ids: readonly number[]) => {
    const gone = new Set(ids);
    set((cur) => ({ threads: cur.threads.filter((t) => !gone.has(t.id)) }));
    scheduleFolders();
  };
  /** Masque ces conversations ; si la conversation ouverte en fait partie, la suivante prend sa place (l'écran ne se vide jamais). */
  const hideAndAdvance = (ids: readonly number[]) => {
    const s = get();
    const gone = new Set(ids);
    const list = visibleThreads(s);
    const rest = list.filter((t) => !gone.has(t.id));
    const next = rest[Math.min(list.findIndex((t) => gone.has(t.id)), rest.length - 1)];
    setHidden(ids, true);
    if (s.selectedId !== null && gone.has(s.selectedId)) get().select(next?.id ?? null);
    else if (s.checked.size > 0) get().clearChecked();
  };
  const fail = (message: string, e: unknown) => get().toast({ message: `${message} — ${String(e)}`, tone: "error", duration: 6000 });

  async function fetchThreads(reset: boolean, silent = false) {
    const seq = ++loadSeq;
    const s = get();
    if (!silent) set({ threadsLoading: true });
    try {
      const rows = await s.backend.listThreads({ ...buildQuery(s), offset: reset ? 0 : s.threads.length });
      if (seq !== loadSeq) return;
      set((cur) => {
        const threads = reset ? rows : [...cur.threads, ...rows];
        // Une conversation disparue (synchro, autre appareil) ne reste pas sélectionnée à l'insu de l'utilisateur.
        const alive = new Set(threads.map((t) => t.id));
        const checked = new Set([...cur.checked].filter((id) => alive.has(id)));
        return { threads, hasMore: rows.length === PAGE, threadsLoading: false, ...(checked.size !== cur.checked.size && { checked: checked.size > 1 ? checked : NO_IDS }) };
      });
    } catch (e) {
      if (seq !== loadSeq) return;
      set({ threadsLoading: false });
      fail("Chargement impossible", e);
    }
  }

  /** Une sélection d'une seule conversation n'en est pas une : c'est la conversation ouverte. */
  const setChecked = (ids: Set<number>, anchor: number | null) => {
    if (ids.size === 1) return get().select([...ids][0]);
    clearTimeout(markReadTimer);
    set((cur) => ({ checked: ids, anchorId: anchor ?? cur.anchorId }));
  };

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
    importOpen: false,
    scheduled: [],
    lock: null,
    scheduledOpen: false,
    backfill: {},
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
    theme: parseTheme(pref.get("theme", "system")),
    colors: parseColors(pref.get("colors", "system")),
    density: pref.get<Density>("density", "comfortable"),
    font: savedFont(),
    previews: pref.get("previews", "on") === "on",

    async init() {
      if (!isTauri()) return get().enterDemo(); // aperçu navigateur : boîte de démonstration
      set({ backend: tauriBackend });
      try {
        // Le verrouillage d'abord : verrouillé, le moteur refuserait tout le reste.
        const lock = await tauriBackend.lockStatus();
        set({ lock });
        if (lock.locked) return void set({ ready: true, threadsLoading: false });
        const accounts = await tauriBackend.listAccounts();
        set({ accounts, needsOnboarding: accounts.length === 0, ready: true });
        if (accounts.length) {
          await loadFolders();
          await refetchAndSelect();
          void get().refreshScheduled();
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

    setImportOpen: (importOpen) => set({ importOpen }),
    setLockStatus: (lock) => set({ lock }),
    async lockNow() {
      await get().backend.lockNow();
      // On efface ce qui est affiché : l'écran de verrouillage ne doit rien laisser derrière lui en mémoire.
      set({ lock: { ...get().lock!, locked: true }, accounts: [], folders: [], threads: [], details: {}, scheduled: [], settingsTab: null, paletteOpen: false, ...NAV_RESET });
    },
    async unlocked() {
      set({ lock: await get().backend.lockStatus(), threadsLoading: true });
      await (get().backend.demo ? get().enterDemo() : get().init());
    },
    setScheduledOpen: (scheduledOpen) => set({ scheduledOpen }),
    async refreshScheduled() {
      try { set({ scheduled: await get().backend.listScheduled() }); } catch { /* conservé */ }
    },

    setBackfill(accountId, remaining) {
      set((s) => (s.backfill[accountId] === remaining ? s : { backfill: { ...s.backfill, [accountId]: remaining } }));
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
      set({ selectedId: id, anchorId: id, checked: NO_IDS });
      if (id === null) return;
      const s = get();
      // Ouvrir une conversation revenue (rappel échu) l'acquitte.
      if (s.threads.find((t) => t.id === id)?.reminderFired) get().clearReminder(id);
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

    /** Cmd/Ctrl+clic : ajoute ou retire une conversation (la conversation ouverte en fait partie d'office). */
    toggleCheck(id) {
      const s = get();
      const next = new Set(s.checked.size > 0 ? s.checked : s.selectedId !== null ? [s.selectedId] : []);
      if (!next.delete(id)) next.add(id);
      setChecked(next, id);
    },

    /** Maj+clic : toutes les conversations entre le point de départ et celle-ci. */
    rangeCheck(id) {
      const s = get();
      const list = visibleThreads(s);
      const from = list.findIndex((t) => t.id === (s.anchorId ?? s.selectedId));
      const to = list.findIndex((t) => t.id === id);
      if (to < 0) return;
      const [lo, hi] = from < 0 ? [to, to] : [Math.min(from, to), Math.max(from, to)];
      setChecked(new Set(list.slice(lo, hi + 1).map((t) => t.id)), s.anchorId ?? s.selectedId);
    },

    checkAll() {
      setChecked(new Set(visibleThreads(get()).map((t) => t.id)), null);
    },

    clearChecked() {
      const id = get().selectedId;
      if (get().checked.size > 0) set({ checked: NO_IDS, anchorId: id });
    },

    move(delta) {
      const s = get();
      const list = visibleThreads(s);
      if (list.length === 0) return;
      const i = list.findIndex((t) => t.id === s.selectedId);
      const next = list[Math.min(list.length - 1, Math.max(0, i < 0 ? 0 : i + delta))];
      if (next) s.select(next.id);
    },

    toggleRead(ids) {
      const list = threadsOf(ids);
      const read = list.some((t) => t.unreadCount > 0); // un lot mixte devient lu ; sinon non lu
      for (const t of list.filter((t) => (t.unreadCount > 0) === read)) {
        const prev = t.unreadCount;
        patchThread(t.id, { unreadCount: read ? 0 : 1 });
        patchDetail(t.id, (d) => ({ ...d, messages: d.messages.map((m, i, a) => ({ ...m, isRead: read ? true : i < a.length - 1 })) }));
        void optimistic(get().backend.setRead(t.id, read), () => patchThread(t.id, { unreadCount: prev }));
      }
    },

    remind(id, kind, at) {
      const prev = get().threads.find((t) => t.id === id);
      if (!prev) return;
      const due = Math.floor(at / 1000);
      // Un report fait sortir la conversation des listes (sauf « Rappels », où elle reste).
      if (kind === "snooze" && get().view.kind !== "reminders") dropThreads([id]);
      else patchThread(id, { reminderKind: kind, reminderDue: due, reminderFired: null });
      void optimistic(get().backend.setReminder(id, kind, due), () => void get().reload({ silent: true }), "Rappel impossible");
    },

    clearReminder(id) {
      const t = get().threads.find((x) => x.id === id);
      if (!t?.reminderKind) return;
      if (get().view.kind === "reminders") dropThreads([id]);
      else patchThread(id, { reminderKind: null, reminderDue: null, reminderFired: null });
      void optimistic(get().backend.clearReminder(id), () => void get().reload({ silent: true }), "Rappel impossible");
    },

    toggleStar(ids) {
      const list = threadsOf(ids);
      const starred = list.some((t) => !t.isStarred); // un lot mixte est suivi en entier ; sinon on ne suit plus
      for (const t of list.filter((t) => t.isStarred !== starred)) {
        patchThread(t.id, { isStarred: starred });
        patchDetail(t.id, (d) => ({ ...d, isStarred: starred }));
        void optimistic(get().backend.setStarred(t.id, starred), () => patchThread(t.id, { isStarred: !starred }), "Action impossible", () => {
          // Dans la vue « Suivis », retirer l'étoile fait sortir la conversation de la liste.
          if (!starred && get().view.kind === "starred") get().select(visibleThreads(get()).find((x) => x.id !== t.id && x.isStarred)?.id ?? null);
        });
      }
    },

    moveTo(ids, to, targetFolderId) {
      const s = get();
      const dest = DESTINATIONS[to];
      // Archivage local : dossier visé (par défaut le premier dossier local, « Archives »).
      const target = to === "local" ? (targetFolderId ?? s.folders.find((f) => f.accountId === LOCAL_ACCOUNT)?.id) : undefined;
      if (to === "local" && target === undefined) return s.toast({ message: dest.failed, tone: "error" });
      const moves = toIds(ids).flatMap((id) => {
        const thread = s.threads.find((t) => t.id === id);
        const from = thread && sourceFolder(s, thread);
        if (!thread || !from) return [];
        // Une conversation locale n'a pas de serveur : elle ne va que vers un autre dossier local.
        if (thread.accountId === LOCAL_ACCOUNT && to !== "local") return [];
        return (to === "local" ? from.id !== target : from.role !== to) ? [{ id, from: from.id }] : [];
      });
      if (moves.length === 0) return s.toast({ message: dest.already });

      hideAndAdvance(moves.map((m) => m.id));
      // Exécuté aussitôt (comme Gmail ou Apple Mail). Archivage local : copie vérifiée, puis suppression sur le serveur.
      type Move = { id: number; from: number };
      const run = to === "local" ? (m: Move) => get().backend.archiveLocal(m.id, m.from, target!) : (m: Move) => get().backend.moveThread(m.id, m.from, to);
      void settleBatch(moves, run).then(({ done, failed, reason }) => {
        setHidden(failed.map((m) => m.id), false);
        if (done.length > 0) {
          dropThreads(done.map((m) => m.id));
          get().toast({ message: done.length === 1 ? dest.done : `${done.length} conversations ${dest.doneMany}` });
        }
        if (failed.length > 0) fail(dest.failed, reason);
      });
    },

    async deleteForever(ids) {
      if (!toIds(ids).some((id) => isPurgeView(actionView(get(), [id])))) return;
      const count = (list: readonly number[]) => list.filter((id) => get().threads.some((t) => t.id === id)).length;
      const n = count(toIds(ids));
      if (n === 0) return;
      const what = n === 1 ? "cette conversation" : `ces ${n} conversations`;
      const local = get().threads.some((t) => toIds(ids).includes(t.id) && t.accountId === LOCAL_ACCOUNT);
      const warning = local ? "Elle n'existe que sur cet ordinateur : il n'y a pas de copie sur un serveur. Cette action est irréversible." : "Cette action est irréversible.";
      if (!(await get().backend.confirm(`Supprimer définitivement ${what} ? ${warning}`, "Supprimer"))) return;
      // L'état a pu changer pendant la boîte de dialogue : on repart de l'état courant.
      const s = get();
      const deletes = toIds(ids).flatMap((id) => {
        const thread = s.threads.find((t) => t.id === id);
        const from = thread && sourceFolder(s, thread);
        return from && isPurgeView(actionView(s, [id])) ? [{ id, from: from.id }] : [];
      });
      if (deletes.length === 0) return;
      hideAndAdvance(deletes.map((d) => d.id));
      const { done, failed, reason } = await settleBatch(deletes, (d) => get().backend.deleteThread(d.id, d.from));
      setHidden(failed.map((d) => d.id), false);
      if (done.length > 0) {
        dropThreads(done.map((d) => d.id));
        get().toast({ message: done.length === 1 ? "Conversation supprimée définitivement" : `${done.length} conversations supprimées définitivement` });
      }
      if (failed.length > 0) fail("Suppression impossible", reason);
    },

    async emptyView() {
      const s = get();
      const view = s.view;
      if (!isEmptiableView(view)) return;
      const folders = s.folders.filter((f) => f.role === view.role && (!s.accountFilter || f.accountId === s.accountFilter));
      const what = view.role === "trash" ? "la corbeille" : "le spam";
      if (folders.length === 0 || !(await s.backend.confirm(`Vider ${what} ? Les conversations seront supprimées définitivement.`, "Vider"))) return;
      try {
        for (const f of folders) await s.backend.emptyFolder(f.id);
        get().toast({ message: view.role === "trash" ? "Corbeille vidée" : "Spam vidé" });
      } catch (e) {
        fail("Impossible de vider", e);
      } finally {
        void get().reload({ silent: true }); // même en cas d'échec : des dossiers ont pu être vidés avant l'erreur
      }
    },

    toast(t) {
      const id = ++toastSeq;
      const toast: Toast = { id, tone: "default", duration: 3200, ...t };
      // Un seul toast à la fois : le dernier remplace le précédent (pas d'empilement bruyant).
      set({ toasts: [toast] });
      clearTimeout(toastTimer);
      toastTimer = setTimeout(() => get().dismissToast(id), toast.duration);
    },
    dismissToast(id) {
      set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
    },

    setPalette: (paletteOpen) => set({ paletteOpen }),
    openSettings: (settingsTab) => set({ settingsTab }),
    setHelp: (helpOpen) => set({ helpOpen }),
    setTheme(theme) {
      pref.set("theme", theme);
      set({ theme });
      renderAppearance(get());
    },
    setColors(colors) {
      pref.set("colors", colors);
      set({ colors });
      renderAppearance(get());
    },
    setDensity(density) {
      pref.set("density", density);
      set({ density });
    },
    setFont(font) {
      pref.set("font", font);
      set({ font });
      void applyFont(font);
    },
    setPreviews(previews) {
      pref.set("previews", previews ? "on" : "off");
      set({ previews });
    },
  };
});
