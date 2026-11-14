import { useComposer } from "@/lib/composer";
import { useMail, type SettingsTab } from "@/lib/store";
import { i18next, resolveLanguage } from "@/lib/i18n";
import { runListBench } from "./dev-bench";

/** Aides de développement (captures d'écran reproductibles), actives uniquement en `pnpm dev` :
 *  ?bench=N (mesure : N conversations synthétiques, défilement jusqu'au bout, résultat dans #bench) · ?lang=fr|en · ?theme=light|dark · ?thread=ID · ?settings=TAB · ?palette=1 · ?help=1 · ?onboarding=1 · ?compose=new|reply|replyAll|forward */
export function applyDevParams() {
  const q = new URLSearchParams(location.search);
  const lang = q.get("lang"); // ?lang=fr|en : langue de l'interface, sans toucher au réglage mémorisé
  if (lang) void i18next.changeLanguage(resolveLanguage(lang));
  const bench = Number(q.get("bench"));
  if (bench > 0) {
    const unsubBench = useMail.subscribe((s) => { if (s.ready && !s.threadsLoading) { unsubBench(); runListBench(bench); } });
    return;
  }
  const theme = q.get("theme");
  if (theme === "light" || theme === "dark") useMail.getState().setTheme(theme);

  // Une fois la boîte chargée (sélection initiale comprise) pour ne pas être écrasé par elle.
  const unsub = useMail.subscribe((s) => {
    if (!s.ready || s.threadsLoading) return;
    unsub();
    const id = Number(q.get("thread"));
    if (id) s.select(id);
    const tab = q.get("settings") as SettingsTab | null;
    useMail.setState({
      ...(tab && { settingsTab: tab }),
      ...(q.get("palette") && { paletteOpen: true }),
      ...(q.get("help") && { helpOpen: true }),
      ...(q.get("onboarding") && { needsOnboarding: true }),
    });
    const compose = q.get("compose");
    if (compose === "new") {
      useComposer.getState().compose();
      if (q.get("colors")) { // ?colors=1 : texte coloré (dont des couleurs foncées) pour juger le thème sombre
        const spans = [["#000080", "navy"], ["#8b0000", "dark red"], ["#dc2626", "red"], ["#16a34a", "green"], ["#2563eb", "blue"], ["#7c3aed", "violet"]];
        useComposer.getState().update({ html: `<p>Neutral, ${spans.map(([c, n]) => `<span style="color:${c}">${n}</span>`).join(", ")}</p>` });
      }
      if (q.get("attach")) useComposer.getState().update({ attachments: [{ id: "1", name: "Devis-refonte-site-v3.pdf", mimeType: "application/pdf", size: 482_000, dataBase64: "" }, { id: "2", name: "photo.jpg", mimeType: "image/jpeg", size: 2_300_000, dataBase64: "" }] });
    }
    else if (compose === "reply" || compose === "replyAll" || compose === "forward") {
      // Le détail de la conversation arrive après la sélection.
      const wait = setInterval(() => {
        if (id && useMail.getState().details[id]) { clearInterval(wait); useComposer.getState().respond(compose); }
      }, 100);
    }
  });
}
