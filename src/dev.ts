import { applyTheme } from "@/lib/theme";
import { useComposer } from "@/lib/composer";
import { useMail, type SettingsTab } from "@/lib/store";

/** Aides de développement (captures d'écran reproductibles), actives uniquement en `pnpm dev` :
 *  ?theme=light|dark · ?thread=ID · ?settings=TAB · ?palette=1 · ?help=1 · ?onboarding=1 · ?compose=new|reply|replyAll|forward */
export function applyDevParams() {
  const q = new URLSearchParams(location.search);
  const theme = q.get("theme");
  if (theme === "light" || theme === "dark") applyTheme(theme);

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
