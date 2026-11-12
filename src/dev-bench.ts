import { useMail } from "@/lib/store";

/** Mesure de la liste : N conversations synthétiques servies par pages de 100, défilement automatique jusqu'à la fin,
 *  puis nombre de nœuds DOM, tas JS et durée écrits dans `#bench` (lu par `edge --dump-dom`, voir CLAUDE.md). */
export function runListBench(n: number) {
  const summary = (i: number) => ({
    id: i + 1, accountId: "pro", subject: `Conversation de mesure n° ${i + 1} : un objet d'une longueur ordinaire`, snippet: "Un aperçu de quelques mots pour la ligne, assez long pour être tronqué à l'affichage dans la liste.",
    lastMessageAt: new Date(Date.now() - i * 420_000).toISOString(), messageCount: 1 + (i % 4), unreadCount: i % 5 === 0 ? 1 : 0,
    isStarred: i % 11 === 0, hasAttachments: i % 3 === 0, fromName: `Expéditeur ${i % 300}`, fromAddr: `exp${i % 300}@example.com`, participants: i % 4 ? "Alice, Bob" : null, folderId: 1, reminderKind: null, reminderDue: null, reminderFired: null,
  });
  const { backend } = useMail.getState();
  useMail.setState({ backend: { ...backend, listThreads: (q) => Promise.resolve(Array.from({ length: Math.max(0, Math.min(q.limit ?? 100, n - (q.offset ?? 0))) }, (_, k) => summary((q.offset ?? 0) + k))) } });
  void useMail.getState().reload();
  const started = performance.now();
  const out = document.body.appendChild(Object.assign(document.createElement("pre"), { id: "bench", textContent: "en cours" }));
  const timer = setInterval(() => {
    const list = document.querySelector<HTMLElement>('[role="listbox"]');
    if (list) list.scrollTop = list.scrollHeight;
    void useMail.getState().loadMore(); // ce que fait l'observateur d'intersection au bas de la liste
    const { threads, hasMore, threadsLoading } = useMail.getState();
    const loaded = threads.length;
    // Tant que les conversations synthétiques ne sont pas arrivées (ou qu'il en reste à charger), on continue de défiler.
    if (!threads[0]?.subject.startsWith("Conversation de mesure") || threadsLoading || (loaded < n && hasMore)) return;
    clearInterval(timer);
    setTimeout(() => {
      const mem = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
      const result = { conversations: n, charges: loaded, noeudsDom: document.getElementsByTagName("*").length, tasJsMo: mem ? +(mem.usedJSHeapSize / 1e6).toFixed(1) : null, dureeMs: Math.round(performance.now() - started), selectionVisible: false };
      // Navigation au clavier : sélectionner une conversation lointaine doit la ramener dans la fenêtre.
      const target = Math.floor(n / 2);
      try { void Promise.resolve(useMail.getState().select(target)).catch(() => {}); } catch { /* le détail synthétique n'existe pas : seule la sélection compte */ }
      setTimeout(() => {
        const row = document.querySelector<HTMLElement>(`[data-thread="${target}"]`);
        const box = document.querySelector<HTMLElement>('[role="listbox"]')?.getBoundingClientRect();
        const r = row?.getBoundingClientRect();
        result.selectionVisible = !!(r && box && r.top >= box.top - 1 && r.bottom <= box.bottom + 1);
        out.textContent = JSON.stringify(result);
      }, 1500);
    }, 1500);
  }, 50);
}
