import i18next, { type Resource } from "i18next";
import { initReactI18next } from "react-i18next";
import { pref } from "./pref";

/** Langues livrées. L'anglais est la langue source des clés et la langue de repli ; le français est tenu au même niveau. */
export const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "fr", label: "Français" },
] as const;
type LangCode = (typeof LANGUAGES)[number]["code"];
/** Réglage : suivre le système, ou une langue précise. */
export type LangPref = "system" | LangCode;

/** Langue effective d'un réglage. « Système » : celle de l'interface web du système (le webview reprend la langue de l'OS,
 *  inutile d'ajouter un plugin) ; une langue non livrée retombe sur l'anglais. */
export function resolveLanguage(p: string, system = typeof navigator === "undefined" ? "en" : navigator.language): LangCode {
  const want = (p === "system" ? system : p).toLowerCase().split("-")[0];
  return LANGUAGES.find((l) => l.code === want)?.code ?? "en";
}

export const parseLangPref = (v: string): LangPref => (v === "system" || LANGUAGES.some((l) => l.code === v) ? (v as LangPref) : "system");

/** Un fichier par langue et par écran : `src/locales/<langue>/<espace>.json` (espace de noms i18next = nom du fichier). */
const resources: Resource = {};
for (const [path, json] of Object.entries(import.meta.glob("../locales/*/*.json", { eager: true, import: "default" }))) {
  const [, lang, file] = /locales\/([^/]+)\/([^/]+)\.json$/.exec(path) ?? [];
  if (lang) (resources[lang] ??= {})[file] = json as Record<string, string>;
}

void i18next.use(initReactI18next).init({
  resources,
  defaultNS: "common",
  ns: Object.keys(resources.en),
  lng: resolveLanguage(pref.get("lang", "system")),
  fallbackLng: "en",
  interpolation: { escapeValue: false }, // React échappe déjà
  initAsync: false, // textes embarqués : prêt avant le premier rendu
  returnNull: false,
});
const syncDocumentLang = () => { if (typeof document !== "undefined") document.documentElement.lang = i18next.language; };
syncDocumentLang();

/** Change la langue de l'interface et la mémorise. */
export async function setLanguage(p: LangPref): Promise<void> {
  pref.set("lang", p);
  await i18next.changeLanguage(resolveLanguage(p));
  syncDocumentLang();
}

/** Langue active, au format attendu par `Intl` (« fr », « en »). */
export const locale = (): string => i18next.language;
export const t = i18next.t.bind(i18next);
export { i18next };
