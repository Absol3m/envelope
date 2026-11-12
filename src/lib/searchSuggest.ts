import { t } from "./i18n";

/** Suggestions de la barre de recherche (opérateurs, valeurs) : logique pure, sans interface. */

export interface Suggestion {
  label: string;
  hint?: string;
  /** Texte qui remplace le mot en cours de saisie. */
  insert: string;
}
export interface Token { start: number; end: number; text: string }

export type Context =
  | { kind: "operator"; token: Token; prefix: string }
  | { kind: "contact" | "value"; token: Token; op: string; prefix: string };

const norm = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/** Mot (délimité par des espaces) sous le curseur. */
export function tokenAt(q: string, caret: number): Token {
  let start = caret;
  while (start > 0 && !/\s/.test(q[start - 1])) start--;
  let end = caret;
  while (end < q.length && !/\s/.test(q[end])) end++;
  return { start, end, text: q.slice(start, end) };
}

const CONTACT_OPS = new Set(["de", "from", "a", "to"]);
const VALUE_OPS = new Set(["avant", "before", "apres", "after", "has", "is", "est", "dans", "in"]);

/** Ce qu'on est en train de taper : un opérateur, ou la valeur d'un opérateur connu (`de:`, `avant:`…). */
export function contextAt(q: string, caret: number): Context | null {
  const token = tokenAt(q, caret);
  const colon = token.text.indexOf(":");
  if (colon < 0) return { kind: "operator", token, prefix: token.text.slice(0, caret - token.start) };
  const op = norm(token.text.slice(0, colon));
  const prefix = token.text.slice(colon + 1).replace(/^"/, "");
  if (CONTACT_OPS.has(op)) return { kind: "contact", token, op, prefix };
  if (VALUE_OPS.has(op)) return { kind: "value", token, op, prefix };
  return null;
}

interface Operator { insert: string; hint: string; flag?: boolean }
/** Opérateurs proposés, écrits dans la langue de l'interface (le moteur de recherche accepte les deux : `de:` comme `from:`). */
const OPERATOR_KEYS = ["from", "to", "subject", "in", "before", "after", "attachment", "unread", "starred"] as const;
const FLAG_OPERATORS = new Set<string>(["attachment", "unread", "starred"]);
export const operators = (): Operator[] => OPERATOR_KEYS.map((k) => ({ insert: t(`search:ops.${k}.insert`), hint: t(`search:ops.${k}.hint`), flag: FLAG_OPERATORS.has(k) }));

export function operatorItems(prefix: string): Suggestion[] {
  const p = norm(prefix);
  return operators().filter((o) => norm(o.insert).startsWith(p)).map((o) => ({ label: o.insert, hint: o.hint, insert: o.flag ? `${o.insert} ` : o.insert }));
}

const pad = (n: number) => String(n).padStart(2, "0");
const ymd = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const ym = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}`;

/** Valeurs proposées après un opérateur de date, `has:` ou `is:` ; `op` sans accent, en minuscules. */
export function valueItems(op: string, prefix: string, now = new Date()): Suggestion[] {
  const key = norm(prefix);
  let items: { label: string; value: string; hint?: string }[];
  if (op === "dans" || op === "in") items = [
    ...(["archive", "sent", "drafts", "inbox", "trash", "spam", "all"] as const).map((k) => ({ label: t(`search:folders.${k}.label`), value: t(`search:folders.${k}.value`) })),
  ];
  else if (op === "has") items = [{ label: "attachment", value: "attachment", hint: t("search:values.attachment") }];
  else if (op === "is" || op === "est") items = [{ label: "unread", value: "unread", hint: t("search:values.unread") }, { label: "starred", value: "starred", hint: t("search:values.starred") }];
  else {
    const lastMonth = new Date(now.getFullYear(), now.getMonth() - 1, 1);
    items = [
      { label: t("search:dates.today"), value: ymd(now), hint: ymd(now) },
      { label: t("search:dates.thisMonth"), value: ym(now), hint: ym(now) },
      { label: t("search:dates.lastMonth"), value: ym(lastMonth), hint: ym(lastMonth) },
      { label: t("search:dates.thisYear"), value: String(now.getFullYear()), hint: String(now.getFullYear()) },
      { label: t("search:dates.lastYear"), value: String(now.getFullYear() - 1), hint: String(now.getFullYear() - 1) },
    ];
  }
  // Tape-t-on déjà une date (« 2026-0… ») ? On filtre sur la valeur ; sinon sur le libellé.
  return items.filter((i) => !key || norm(i.value).startsWith(key) || norm(i.label).startsWith(key)).map((i) => ({ label: i.label, hint: i.hint, insert: `${i.value} ` }));
}

/** Remplace le mot `token` de `q` par `lead` + `insert` (`lead` : l'opérateur déjà tapé, `de:`) ;
 *  renvoie le nouveau texte et la position du curseur. Pas d'espace en trop si un autre mot suit. */
export function applySuggestion(q: string, token: Token, lead: string, insert: string): { text: string; caret: number } {
  const value = /^\s/.test(q.slice(token.end)) ? `${lead}${insert}`.trimEnd() : `${lead}${insert}`;
  return { text: q.slice(0, token.start) + value + q.slice(token.end), caret: token.start + value.length };
}
