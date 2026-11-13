import { locale, t } from "./i18n";

const DAY = 86_400_000;

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** Formateurs `Intl` de la langue active, gardés par langue et par options (leur construction est coûteuse). */
const formatters = new Map<string, Intl.DateTimeFormat | Intl.RelativeTimeFormat | Intl.NumberFormat>();
function cached<F extends Intl.DateTimeFormat | Intl.RelativeTimeFormat | Intl.NumberFormat>(kind: string, options: object, make: (loc: string) => F): F {
  const key = `${locale()}|${kind}|${JSON.stringify(options)}`;
  let f = formatters.get(key);
  if (!f) formatters.set(key, (f = make(locale())));
  return f as F;
}
const dtf = (options: Intl.DateTimeFormatOptions) => cached("d", options, (l) => new Intl.DateTimeFormat(l, options));
const time = () => dtf({ hour: "2-digit", minute: "2-digit" });
const weekdayShort = () => dtf({ weekday: "short" });
const dayMonth = () => dtf({ day: "numeric", month: "short" });
const dayMonthYear = () => dtf({ day: "numeric", month: "short", year: "2-digit" });
const full = () => dtf({ weekday: "long", day: "numeric", month: "long", year: "numeric", hour: "2-digit", minute: "2-digit" });
const scheduled = () => dtf({ weekday: "long", day: "numeric", month: "long", hour: "numeric", minute: "2-digit" });
const monthYear = () => dtf({ month: "long", year: "numeric" });
const fullShort = () => dtf({ dateStyle: "full", timeStyle: "short" });
const weekdayTime = () => dtf({ weekday: "long", hour: "numeric", minute: "2-digit" });

/** Date et heure d'un envoi programmé (millisecondes) : « lundi 16 novembre à 08:00 ». */
/** « lundi, 16 novembre 2026 à 08:00 » (date longue d'un message cité). */
export const fullDateTime = (iso: string) => fullShort().format(new Date(iso));
/** « lundi 08:00 » (créneaux proposés par « Envoyer plus tard »). */
export const weekdayTimeOf = (d: Date) => weekdayTime().format(d);

export const scheduledAt = (ms: number) => scheduled().format(ms);

/** Heure du jour, « Hier », jour de semaine, puis date : le plus court lisible. */
export function listDate(iso: string, now = new Date()): string {
  const d = new Date(iso);
  const days = Math.round((startOfDay(now) - startOfDay(d)) / DAY);
  if (days <= 0) return time().format(d);
  if (days === 1) return t("common:time.yesterday");
  if (days < 7) return weekdayShort().format(d).replace(".", "");
  return (d.getFullYear() === now.getFullYear() ? dayMonth() : dayMonthYear()).format(d).replace(".", "");
}

export const fullDate = (iso: string) => full().format(new Date(iso));

/** Intitulé de groupe dans la liste : Aujourd'hui, Hier, 7 derniers jours, puis par mois. */
export function groupLabel(iso: string, now = new Date()): string {
  const d = new Date(iso);
  const days = Math.round((startOfDay(now) - startOfDay(d)) / DAY);
  if (days <= 0) return t("common:time.today");
  if (days === 1) return t("common:time.yesterday");
  if (days < 7) return t("common:time.thisWeek");
  if (d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth()) return t("common:time.thisMonth");
  const label = monthYear().format(d);
  return label.charAt(0).toUpperCase() + label.slice(1);
}

/** « il y a 5 min » : `Intl.RelativeTimeFormat` (forme abrégée) plutôt que des phrases à traduire une à une. */
export function relativeTime(iso: string | null, now = Date.now()): string {
  if (!iso) return t("common:time.never");
  const s = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
  if (s < 45) return t("common:time.justNow");
  const rtf = cached("r", {}, (l) => new Intl.RelativeTimeFormat(l, { numeric: "always", style: "short" }));
  if (s < 3600) return rtf.format(-Math.round(s / 60), "minute");
  if (s < 86_400) return rtf.format(-Math.round(s / 3600), "hour");
  return rtf.format(-Math.round(s / 86_400), "day");
}

/** Nombre selon les usages de la langue active (séparateurs de milliers et décimal). */
export const formatNumber = (n: number, maximumFractionDigits = 0) => cached("n", { maximumFractionDigits }, (l) => new Intl.NumberFormat(l, { maximumFractionDigits })).format(n);

export function fileSize(bytes: number): string {
  if (bytes < 1024) return t("common:size.bytes", { n: bytes });
  if (bytes < 1024 * 1024) return t("common:size.kilobytes", { n: Math.round(bytes / 1024) });
  return t("common:size.megabytes", { n: formatNumber(bytes / 1024 / 1024, 1) });
}

/** « Nom <adresse> » → nom affichable (repli : partie locale de l'adresse). */
export function displayName(name: string | null | undefined, addr: string | null | undefined): string {
  if (name?.trim()) return name.trim();
  const a = addr ?? "";
  return a.split("@")[0] || a || t("common:unknown");
}

export function initials(name: string): string {
  const parts = name.replace(/[<>()"]/g, "").trim().split(/[\s.\-_]+/).filter(Boolean);
  if (parts.length === 0) return "?";
  const first = parts[0][0] ?? "";
  const last = parts.length > 1 ? parts[parts.length - 1][0] : "";
  return (first + last).toUpperCase();
}

/** Teinte stable (0–359) dérivée d'une chaîne : même expéditeur, même couleur d'avatar. */
export function hue(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) h = Math.imul(h ^ seed.charCodeAt(i), 16777619);
  return Math.abs(h) % 360;
}

/** Fond d'avatar : dégradé stable par expéditeur (même adresse, même couleur). */
export function avatarBackground(name: string | null | undefined, addr: string | null | undefined): string {
  const h = hue((addr ?? displayName(name, addr)).toLowerCase());
  return `linear-gradient(135deg, hsl(${h} 62% 58%), hsl(${(h + 38) % 360} 66% 46%))`;
}
