import { isTauri, systemAppearance, type SystemAppearance } from "./api";
import { isMac, isWindows } from "./platform";

/** Clair / sombre. */
export type ThemePref = "system" | "light" | "dark";
/** Couleurs : celles du système (palette de l'OS ou du bureau, accent) ou celles d'Envelope. */
export type ColorsPref = "system" | "envelope";
export interface AppearancePrefs {
  theme: ThemePref;
  colors: ColorsPref;
}

export const parseTheme = (v: string): ThemePref => (v === "light" || v === "dark" ? v : "system");
export const parseColors = (v: string): ColorsPref => (v === "envelope" ? "envelope" : "system");

declare global {
  interface Window {
    /** Faits fixes du système, injectés par Rust avant le premier affichage (lib.rs, `main_window`). */
    __ENVELOPE_HOST__?: { palette: string; material: "mica" | "vibrancy" | null };
  }
}

/** Hors Tauri (aperçu navigateur), palette devinée d'après le user agent, sans matière. */
const host = window.__ENVELOPE_HOST__ ?? { palette: isMac ? "macos" : isWindows ? "windows" : "gnome", material: null };
/** Réglages changeants du système (accent, schéma KDE…), relus au retour au premier plan. */
let system: SystemAppearance | null = null;

const darkQuery = () => window.matchMedia("(prefers-color-scheme: dark)");

/** Luminance perçue d'un `#rrggbb`, de 0 à 1. */
function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  return (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 0xff) + 0.114 * (n & 0xff)) / 255;
}

function setVars(vars: Record<string, string | null | undefined>) {
  const style = document.documentElement.style;
  for (const [name, value] of Object.entries(vars)) {
    if (value) style.setProperty(name, value);
    else style.removeProperty(name);
  }
}

/** Pose thème, palette, matière et accent sur `<html>` (les couleurs elles-mêmes sont dans index.css). */
export function renderAppearance({ theme, colors }: AppearancePrefs) {
  const root = document.documentElement;
  const scheme = system?.palette;
  const systemDark = system?.dark ?? (scheme ? luminance(scheme.window) < 0.5 : darkQuery().matches);
  const dark = theme === "system" ? systemDark : theme === "dark";
  const native = colors === "system";

  root.dataset.theme = dark ? "dark" : "light";
  root.dataset.palette = native ? host.palette : "envelope";
  if (native && host.material) root.dataset.material = host.material;
  else delete root.dataset.material;

  const accent = native ? (dark ? system?.accentDark : system?.accentLight) : null;
  // Le schéma KDE de l'utilisateur est clair ou sombre : il ne vaut que pour ce thème-là.
  const kde = native && dark === systemDark ? scheme : null;
  setVars({
    "--accent": accent,
    // Texte noir sur un accent clair (jaune, bleu clair de Windows en sombre).
    "--accent-fg": accent && (luminance(accent) > 0.62 ? "#000000" : "#ffffff"),
    "--sys-window": kde?.window,
    "--sys-view": kde?.view,
    "--sys-fg": kde?.fg,
    "--sys-fg-muted": kde?.fgMuted,
  });
  // `null` : la fenêtre suit le système. Sous Linux, le bureau (portail) fait foi plutôt que GTK.
  const linux = host.palette === "gnome" || host.palette === "kde";
  syncWindowTheme(theme === "system" && !linux ? null : dark ? "dark" : "light");
}

let windowTheme: string | null | undefined;
/** Barre de titre native, feux tricolores (macOS) et matière assortis au thème. */
function syncWindowTheme(theme: "light" | "dark" | null) {
  if (!isTauri() || theme === windowTheme) return;
  windowTheme = theme;
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(theme))
    .catch(() => { /* non pris en charge : la fenêtre garde le thème du système */ });
}

/** Suit l'apparence du système : rendu immédiat (avant le premier affichage), puis à chaque
 *  changement clair/sombre et au retour de l'application au premier plan (accent ou schéma
 *  changés dans les réglages du système entre-temps). */
export function watchSystemAppearance(prefs: () => AppearancePrefs) {
  const render = () => renderAppearance(prefs());
  render();
  if (!isTauri()) {
    darkQuery().addEventListener("change", render);
    return;
  }
  let pending: Promise<void> | null = null;
  const refresh = () =>
    (pending ??= systemAppearance()
      .then((next) => {
        if (JSON.stringify(next) === JSON.stringify(system)) return;
        system = next;
        render();
      })
      .catch(() => { /* lecture impossible : palette de base sans accent du système */ })
      .finally(() => (pending = null)));
  void refresh();
  darkQuery().addEventListener("change", () => {
    render();
    void refresh();
  });
  void import("@tauri-apps/api/window").then(({ getCurrentWindow }) =>
    getCurrentWindow().onFocusChanged(({ payload: focused }) => focused && void refresh()),
  );
}
