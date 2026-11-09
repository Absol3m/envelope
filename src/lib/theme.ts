export type ThemePref = "system" | "light" | "dark";
const KEY = "envelope.theme";

export function loadTheme(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

export function resolveTheme(pref: ThemePref): "light" | "dark" {
  if (pref !== "system") return pref;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(pref: ThemePref) {
  document.documentElement.dataset.theme = resolveTheme(pref);
  try {
    localStorage.setItem(KEY, pref);
  } catch {
    /* stockage indisponible : le thème n'est simplement pas mémorisé */
  }
}
