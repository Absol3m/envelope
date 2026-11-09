import { applyTheme } from "@/lib/theme";
import { useMail, type SettingsTab } from "@/lib/store";

/** Aides de développement (captures d'écran reproductibles), actives uniquement en `pnpm dev` :
 *  ?theme=light|dark · ?thread=ID · ?settings=TAB · ?palette=1 · ?help=1 · ?onboarding=1 */
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
  });
}
