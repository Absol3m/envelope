/** Recherche de la démo : port de `parse_search` et des filtres de message du moteur (`envelope-core/src/search.rs`).
 *  Le test de contrat (`listContract.test.ts`) rejoue le même scénario que le moteur : toute évolution des opérateurs
 *  ajoute un cas à `src-tauri/tests/contract/list_threads.json`. Écarts connus : `LIKE` du moteur ne replie la casse
 *  que pour l'ASCII (ici toute la casse) ; la tokenisation FTS5 (`unicode61`) est approchée par « lettres et chiffres ». */
import type { FolderRole } from "./bindings/FolderRole";

export interface SearchSpec {
  text: string;
  from: string[];
  to: string[];
  subject: string[];
  before?: string;
  after?: string;
  folders: string[];
  anywhere: boolean;
  attachment: boolean;
  unread: boolean;
  starred: boolean;
}

/** Minuscules sans accents (comparaison de mots et de noms de dossiers). */
export const fold = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

const FOLDER_WORDS: Record<string, FolderRole> = {
  inbox: "inbox", reception: "inbox", boite: "inbox",
  archive: "archive", archives: "archive",
  sent: "sent", envoye: "sent", envoyes: "sent",
  drafts: "drafts", brouillon: "drafts", brouillons: "drafts",
  trash: "trash", corbeille: "trash",
  junk: "junk", spam: "junk", indesirable: "junk", indesirables: "junk",
};

const validDate = (s: string) => /^\d{4}(-(0[1-9]|1[0-2])(-(0[1-9]|[12]\d|3[01]))?)?$/.test(s);

/** Découpe sur les espaces hors guillemets ; les guillemets sont retirés. */
function tokens(input: string): string[] {
  const out: string[] = [];
  let cur = "", quoted = false;
  for (const c of input) {
    if (c === '"') quoted = !quoted;
    else if (/\s/.test(c) && !quoted) { if (cur) out.push(cur); cur = ""; }
    else cur += c;
  }
  if (cur) out.push(cur);
  return out;
}

const ANYWHERE = new Set(["tout", "partout", "anywhere", "all"]);
const UNREAD = new Set(["unread", "nonlu", "non-lu"]);
const STARRED = new Set(["starred", "suivi", "favori"]);

function applyOperator(spec: SearchSpec, tok: string): boolean {
  const i = tok.indexOf(":");
  if (i < 0) {
    const w = tok.toLowerCase();
    if (w === "pj") spec.attachment = true;
    else if (UNREAD.has(w)) spec.unread = true;
    else if (STARRED.has(w)) spec.starred = true;
    else return false;
    return true;
  }
  const value = tok.slice(i + 1).trim();
  if (!value) return false;
  const lower = value.toLowerCase();
  switch (tok.slice(0, i).toLowerCase()) {
    case "de": case "from": spec.from.push(value); return true;
    case "a": case "à": case "to": spec.to.push(value); return true;
    case "objet": case "subject": spec.subject.push(value); return true;
    case "dans": case "in":
      if (ANYWHERE.has(fold(value))) spec.anywhere = true; else spec.folders.push(value);
      return true;
    case "avant": case "before": if (!validDate(value)) return false; spec.before = value; return true;
    case "apres": case "après": case "after": if (!validDate(value)) return false; spec.after = value; return true;
    case "has": if (lower !== "attachment" && lower !== "pj") return false; spec.attachment = true; return true;
    case "is": case "est":
      if (UNREAD.has(lower)) spec.unread = true;
      else if (STARRED.has(lower)) spec.starred = true;
      else return false;
      return true;
    default: return false;
  }
}

export function parseSearch(input: string): SearchSpec {
  const spec: SearchSpec = { text: "", from: [], to: [], subject: [], folders: [], anywhere: false, attachment: false, unread: false, starred: false };
  const free: string[] = [];
  for (const tok of tokens(input)) if (!applyOperator(spec, tok)) free.push(tok);
  spec.text = free.join(" ");
  return spec;
}

/** Mots (lettres et chiffres) d'un texte, sans accents ni casse (tokenisation `unicode61` de FTS5, approchée). */
const wordsOf = (s: string) => fold(s).split(/[^\p{L}\p{N}]+/u).filter(Boolean);

/** Mots libres → termes FTS5 : chaque terme est une phrase de mots dont le dernier est un préfixe (`"mot"*`), tous requis ;
 *  `null` : rien d'interrogeable. */
export function ftsTerms(text: string): string[][] | null {
  const terms = text.split(/\s+/).map((t) => t.replaceAll('"', "")).filter((t) => /[\p{L}\p{N}]/u.test(t)).map(wordsOf);
  return terms.length ? terms : null;
}

/** La phrase `term` (le dernier mot en préfixe) figure-t-elle, mots contigus, dans `tokens` ? */
const phraseIn = (col: string[], term: string[]) =>
  col.some((_, i) => term.every((w, j) => (j === term.length - 1 ? col[i + j]?.startsWith(w) : col[i + j] === w)));

export interface SearchableMessage {
  subject: string;
  fromName: string | null;
  fromAddr: string;
  to: { name: string | null; address: string }[];
  cc: { name: string | null; address: string }[];
  date: string;
  body: string;
  isRead: boolean;
  isStarred: boolean;
  hasAttachments: boolean;
  /** Rôle et nom du dossier du message. */
  folderRole: FolderRole;
  folderName: string;
}

const has = (hay: string, lowerNeedle: string) => hay.toLowerCase().includes(lowerNeedle);

/** Prépare une requête : termes FTS, aiguilles en minuscules et rôles de dossiers sont calculés une fois, pas par message.
 *  Le prédicat rendu dit si un message satisfait tous les critères (comme `push_message_filters`). */
export function compileSpec(spec: SearchSpec): (m: SearchableMessage) => boolean {
  const wantsText = spec.text.trim() !== "";
  const terms = wantsText ? ftsTerms(spec.text) : null;
  const [from, to, subject] = [spec.from, spec.to, spec.subject].map((l) => l.map((v) => v.toLowerCase()));
  const folderRoles = spec.folders.map((v) => FOLDER_WORDS[fold(v)]);
  const people = (xs: SearchableMessage["to"]) => xs.map((a) => `${a.name ?? ""} ${a.address}`).join(" ");
  return (m) => {
    if (wantsText) {
      if (!terms) return false; // rien d'interrogeable : aucun résultat (texte vide = pas de filtre)
      // FTS5 : un terme se cherche dans une colonne (objet, nom, adresse, corps) ; tous les termes dans le même message.
      const columns = [m.subject, m.fromName ?? "", m.fromAddr, m.body].map(wordsOf);
      if (!terms.every((t) => columns.some((c) => phraseIn(c, t)))) return false;
    }
    if (spec.folders.length === 0) {
      if (!spec.anywhere && (m.folderRole === "trash" || m.folderRole === "junk")) return false;
    } else if (!spec.folders.some((v, i) => (folderRoles[i] ? m.folderRole === folderRoles[i] : has(m.folderName, v.toLowerCase())))) return false;
    if (!from.every((v) => has(m.fromAddr, v) || has(m.fromName ?? "", v))) return false;
    if (to.length) {
      const [t, c] = [people(m.to), people(m.cc)];
      if (!to.every((v) => has(t, v) || has(c, v))) return false;
    }
    if (!subject.every((v) => has(m.subject, v))) return false;
    if (spec.before !== undefined && !(m.date < spec.before)) return false;
    if (spec.after !== undefined && !(m.date >= spec.after)) return false;
    if (spec.attachment && !m.hasAttachments) return false;
    if (spec.unread && m.isRead) return false;
    if (spec.starred && !m.isStarred) return false;
    return true;
  };
}
