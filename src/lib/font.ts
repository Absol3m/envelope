import { pref } from "./pref";
import { t } from "./i18n";

/** Police de l'interface : celle du système par défaut, ou une police libre (OFL) chargée à la demande,
 *  ou le nom d'une police installée sur la machine. */
export type FontPref = "system" | "inter" | "source-sans" | "plex-sans" | "atkinson" | "noto-sans" | `custom:${string}`;

/** Police du système : SF Pro sur macOS, Segoe UI sur Windows, la police du bureau sous Linux. */
export const SYSTEM_STACK = 'system-ui, -apple-system, "Segoe UI Variable Text", "Segoe UI", ui-sans-serif, sans-serif';

interface FreeFont {
  id: Exclude<FontPref, "system" | `custom:${string}`>;
  label: string;
  /** Nom de la famille déclarée par le paquet Fontsource. */
  family: string;
  /** Ce qui distingue cette police ; sert d'aide dans les réglages. */
  note: string;
  /** Chargée à la première utilisation : rien n'est téléchargé tant qu'on ne la choisit pas. */
  load: () => Promise<unknown>;
  /** Variantes typographiques propres à la police (jeux stylistiques), sinon aucune. */
  features?: string;
}

/** Polices libres proposées (licence SIL OFL 1.1), embarquées sous forme de paquets Fontsource. */
export const FREE_FONTS: readonly FreeFont[] = [
  { id: "inter", label: "Inter", family: '"Inter Variable"', get note() { return t("settings:font.notes.inter"); }, load: () => import("@fontsource-variable/inter"), features: '"cv11", "ss03"' },
  { id: "source-sans", label: "Source Sans 3", family: '"Source Sans 3 Variable"', get note() { return t("settings:font.notes.source-sans"); }, load: () => import("@fontsource-variable/source-sans-3") },
  { id: "plex-sans", label: "IBM Plex Sans", family: '"IBM Plex Sans Variable"', get note() { return t("settings:font.notes.plex-sans"); }, load: () => import("@fontsource-variable/ibm-plex-sans") },
  { id: "atkinson", label: "Atkinson Hyperlegible Next", family: '"Atkinson Hyperlegible Next Variable"', get note() { return t("settings:font.notes.atkinson"); }, load: () => import("@fontsource-variable/atkinson-hyperlegible-next") },
  { id: "noto-sans", label: "Noto Sans", family: '"Noto Sans Variable"', get note() { return t("settings:font.notes.noto-sans"); }, load: () => import("@fontsource-variable/noto-sans") },
];

const MAX_CUSTOM = 60;

/** Nom de police saisi : lettres, chiffres, espaces, points, tirets. Tout le reste est refusé, pour qu'un
 *  nom ne puisse jamais sortir des guillemets de la déclaration CSS. `null` si invalide ou vide. */
export function customName(raw: string): string | null {
  const name = raw.trim().replace(/\s+/g, " ");
  return name.length > 0 && name.length <= MAX_CUSTOM && /^[\p{L}\p{N} .\-]+$/u.test(name) ? name : null;
}

export function parseFont(v: string): FontPref {
  if (FREE_FONTS.some((f) => f.id === v)) return v as FontPref;
  if (v.startsWith("custom:") && customName(v.slice(7))) return v as FontPref;
  return "system";
}

/** Pile de polices et variantes typographiques d'un réglage. Une police absente retombe sur celle du système. */
export function fontStack(font: FontPref): { stack: string; features: string } {
  if (font.startsWith("custom:")) {
    const name = customName(font.slice(7));
    return { stack: name ? `"${name}", ${SYSTEM_STACK}` : SYSTEM_STACK, features: "normal" };
  }
  const free = FREE_FONTS.find((f) => f.id === font);
  return free ? { stack: `${free.family}, ${SYSTEM_STACK}`, features: free.features ?? "normal" } : { stack: SYSTEM_STACK, features: "normal" };
}

/** Applique la police : la pile tout de suite, la police libre dès qu'elle est chargée. */
export async function applyFont(font: FontPref, root: HTMLElement = document.documentElement) {
  const { stack, features } = fontStack(font);
  root.style.setProperty("--font-sans", stack);
  root.style.setProperty("--font-features", features);
  const free = FREE_FONTS.find((f) => f.id === font);
  try {
    await free?.load();
  } catch {
    /* hors ligne ou paquet absent : la police du système reste utilisée */
  }
}

export const savedFont = (): FontPref => parseFont(pref.get("font", "system"));
