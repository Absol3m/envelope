import { sanitizeFragment } from "./sanitize";
import type { Account, Addr, DetailMessage, DraftData, OutgoingMessage, ThreadDetail } from "./api";

export type ComposeMode = "new" | "reply" | "replyAll" | "forward";

export interface Recipient {
  address: string;
  name?: string | null;
}
/** Pièce jointe en mémoire (contenu en base64, jamais écrit sur disque ni dans le brouillon local). */
export interface AttachmentDraft {
  id: string;
  name: string;
  mimeType: string;
  size: number;
  dataBase64: string;
}
/** Même limite que le serveur d'envoi (Rust, `smtp.rs`) : à garder alignées. 25 Mo cumulés, 20 fichiers. */
export const MAX_ATTACHMENTS_BYTES = 25 * 1024 * 1024;
export const MAX_ATTACHMENTS = 20;

export interface Draft {
  /** Identifie le brouillon : l'éditeur est recréé quand il change. */
  id: string;
  accountId: string;
  /** Alias d'envoi ; absent = l'adresse du compte. */
  fromEmail?: string;
  mode: ComposeMode;
  to: Recipient[];
  cc: Recipient[];
  bcc: Recipient[];
  subject: string;
  /** Corps au format HTML produit par l'éditeur riche. */
  html: string;
  attachments: AttachmentDraft[];
  /** Brouillon relu depuis le stockage local : noms des pièces jointes qui n'y sont pas conservées. */
  lostAttachments?: string[];
  inReplyTo?: string;
  references: string[];
}

const EMAIL_RE = /^[^\s@<>,;"]+@[^\s@<>,;"]+\.[^\s@<>,;"]{2,}$/;
export const isEmail = (s: string) => EMAIL_RE.test(s.trim());

/** « Nom <a@b.fr> », « a@b.fr » ou une liste séparée par , ou ; → destinataires valides + fragments rejetés. */
export function parseRecipients(text: string): { ok: Recipient[]; rejected: string[] } {
  const ok: Recipient[] = [];
  const rejected: string[] = [];
  // Découpe sur , ou ; hors des guillemets et des chevrons.
  const parts: string[] = [];
  let cur = "", quoted = false, angled = false;
  for (const ch of text) {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch === "<") angled = true;
    else if (!quoted && ch === ">") angled = false;
    if (!quoted && !angled && (ch === "," || ch === ";" || ch === "\n")) { parts.push(cur); cur = ""; } else cur += ch;
  }
  parts.push(cur);
  for (const raw of parts.map((p) => p.trim()).filter(Boolean)) {
    const m = raw.match(/^(.*?)<\s*([^<>]+?)\s*>$/);
    const address = (m ? m[2] : raw).trim();
    if (!isEmail(address)) { rejected.push(raw); continue; }
    const name = m?.[1].trim().replace(/^"(.*)"$/, "$1").trim();
    ok.push({ address, name: name || null });
  }
  return { ok, rejected };
}

/** Forme transmise au serveur : le nom est omis s'il contiendrait des caractères spéciaux. */
export function formatRecipient(r: Recipient): string {
  const name = r.name?.trim();
  return name && /^[\p{L}\p{N} .'’-]+$/u.test(name) ? `${name} <${r.address}>` : r.address;
}

export function toOutgoing(d: Draft): OutgoingMessage {
  return {
    to: d.to.map(formatRecipient),
    cc: d.cc.map(formatRecipient),
    bcc: d.bcc.map(formatRecipient),
    subject: d.subject.trim(),
    bodyText: htmlToPlain(d.html),
    bodyHtml: emailHtml(d.html),
    inReplyTo: d.inReplyTo,
    references: d.references,
    attachments: d.attachments.map((a) => ({ filename: a.name, mimeType: a.mimeType, dataBase64: a.dataBase64 })),
    draftId: d.id,
    fromEmail: d.fromEmail,
  };
}

/** Vérifie qu'ajouter `files` reste dans les limites ; renvoie le message d'erreur sinon. */
export function attachmentError(current: AttachmentDraft[], files: { name: string; size: number }[]): string | null {
  if (current.length + files.length > MAX_ATTACHMENTS) return `Trop de pièces jointes (maximum ${MAX_ATTACHMENTS}).`;
  const total = [...current, ...files].reduce((n, f) => n + f.size, 0);
  if (total > MAX_ATTACHMENTS_BYTES) return `Pièces jointes trop volumineuses : ${Math.round(MAX_ATTACHMENTS_BYTES / 1024 / 1024)} Mo au total au maximum.`;
  return null;
}

/** Contenu d'un fichier en base64 (sans préfixe data:). */
export function readAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => {
      const url = String(r.result);
      resolve(url.slice(url.indexOf(",") + 1));
    };
    r.onerror = () => reject(new Error(`lecture impossible : ${file.name}`));
    r.readAsDataURL(file);
  });
}

export const isBlank = (d: Draft) => !d.to.length && !d.cc.length && !d.bcc.length && !d.subject.trim() && !d.attachments.length && !htmlToPlain(d.html);

export function emptyDraft(accountId: string, signature = ""): Draft {
  return { id: crypto.randomUUID(), accountId, mode: "new", to: [], cc: [], bcc: [], subject: "", html: signature ? `<p></p>${signatureHtml(signature)}` : "", attachments: [], references: [] };
}

/** Brouillon relu depuis le serveur → brouillon du composer (le HTML venu du serveur est assaini). */
/** Brouillon relu depuis le serveur → brouillon du composer (le HTML venu du serveur est assaini). */
/** Message d'un envoi programmé annulé → brouillon à rouvrir (même forme qu'un brouillon relu du serveur). */
export const draftFromQueued = (accountId: string, m: OutgoingMessage): Draft =>
  draftFromServer({ id: m.draftId ?? crypto.randomUUID(), accountId, to: m.to, cc: m.cc, bcc: m.bcc, subject: m.subject, bodyHtml: m.bodyHtml ?? null, bodyText: m.bodyText, inReplyTo: m.inReplyTo ?? null, references: m.references, attachments: m.attachments, fromEmail: m.fromEmail ?? "" });

export function draftFromServer(d: DraftData): Draft {
  const recipients = (list: string[]) => parseRecipients(list.join(", ")).ok;
  return {
    id: d.id,
    accountId: d.accountId,
    mode: d.inReplyTo ? "reply" : "new",
    to: recipients(d.to),
    cc: recipients(d.cc),
    bcc: recipients(d.bcc),
    subject: d.subject,
    html: d.bodyHtml?.trim() ? sanitizeFragment(d.bodyHtml) : textToHtml(d.bodyText),
    attachments: d.attachments.map((a) => ({ id: crypto.randomUUID(), name: a.filename, mimeType: a.mimeType, size: Math.floor((a.dataBase64.length * 3) / 4), dataBase64: a.dataBase64 })),
    inReplyTo: d.inReplyTo ?? undefined,
    references: d.references,
  };
}



const PREFIX = /^\s*((re|tr|fwd?|fw)\s*:\s*)+/i;
export const withPrefix = (prefix: "Re" | "Fwd", subject: string) => `${prefix}: ${subject.replace(PREFIX, "").trim()}`.trim();

const escapeHtml = (t: string) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** Signature (texte brut) → bloc HTML précédé du séparateur usuel « -- ». Vide si pas de signature. */
export function signatureHtml(signature: string): string {
  const text = signature.trim();
  return text ? `<p>--<br>${escapeHtml(text).replace(/\r?\n/g, "<br>")}</p>` : "";
}

/** Change de signature quand on change de compte expéditeur : seulement si l'ancienne est encore
 *  intacte dans le corps (une signature retouchée à la main n'est pas écrasée) ou si le corps est vide. */
export function swapSignature(html: string, from: string, to: string): string {
  const old = signatureHtml(from);
  if (old && html.includes(old)) return html.replace(old, signatureHtml(to));
  return !htmlToPlain(html) && to.trim() ? `<p></p>${signatureHtml(to)}` : html;
}

/** Texte brut → paragraphes HTML (lignes vides = nouveaux paragraphes). */
export function textToHtml(text: string): string {
  return text
    .trim()
    .split(/\r?\n{2,}/)
    .map((p) => `<p>${escapeHtml(p).replace(/\r?\n/g, "<br>")}</p>`)
    .join("");
}

/** HTML de l'éditeur → texte brut lisible : listes à tirets, citations préfixées par « > », liens avec leur adresse. */
export function htmlToPlain(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const render = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
    if (node.nodeType !== Node.ELEMENT_NODE) return "";
    const el = node as HTMLElement;
    const inner = () => Array.from(el.childNodes).map(render).join("");
    switch (el.tagName) {
      case "BR": return "\n";
      case "P": case "DIV": case "H1": case "H2": case "H3": case "H4": return `${inner()}\n\n`;
      case "UL": case "OL": return `${inner()}\n`;
      case "LI": {
        const n = el.parentElement?.tagName === "OL" ? `${Array.from(el.parentElement.children).indexOf(el) + 1}. ` : "- ";
        return `${n}${inner().trim()}\n`;
      }
      case "BLOCKQUOTE": return `${inner().trim().split("\n").map((l) => (l ? `> ${l}` : ">")).join("\n")}\n\n`;
      case "A": {
        const href = el.getAttribute("href") ?? "";
        const text = inner();
        return href && href !== text && href !== `mailto:${text}` ? `${text} (${href})` : text;
      }
      default: return inner();
    }
  };
  return render(doc.body).replace(/\n{3,}/g, "\n\n").trim();
}

/** Applique un modèle : son corps passe avant la signature et les citations, son objet ne remplace pas un objet déjà saisi. */
export function applyTemplate(d: Draft, t: { subject: string; bodyHtml: string }): Draft {
  return { ...d, subject: d.subject.trim() ? d.subject : t.subject, html: t.bodyHtml + d.html.replace(/^<p><\/p>/, "") };
}

/** Corps à enregistrer comme modèle : le texte de l'auteur, sans la signature du compte ni la citation d'une réponse. */
export function templateBody(d: Draft, signature: string): string {
  let html = d.html;
  const sig = signatureHtml(signature);
  if (sig) html = html.replace(sig, "");
  const quote = html.indexOf("<blockquote");
  if (quote >= 0) html = html.slice(0, Math.max(0, html.lastIndexOf("<p>", quote))); // l'en-tête « X a écrit » précède la citation
  return html.replace(/^(<p><\/p>)+/, "");
}

/** Mots qui annoncent une pièce jointe (français, anglais) ; « joint » seul est exclu (le joint de culasse). */
const ATTACHMENT_WORDS = /(?<![\p{L}\p{N}])(?:pi[eè]ces?\s+jointes?|ci[-\s]?joint(?:e|es|s)?|(?:je|nous)\s+(?:vous\s+)?(?:joins|joignons)|attach(?:ed|ment|ments|ing)|enclosed)(?![\p{L}\p{N}])/iu;

/** Le texte écrit mentionne une pièce jointe alors qu'il n'y en a aucune : oubli probable. Les citations
 *  (réponse, transfert) et la signature (après « -- ») ne comptent pas, c'est le texte de l'auteur qui parle. */
export function forgotAttachment(d: Draft): boolean {
  if (d.attachments.length) return false;
  const lines = htmlToPlain(d.html).split("\n");
  const end = lines.findIndex((l) => l.trim() === "--");
  const own = (end < 0 ? lines : lines.slice(0, end)).filter((l) => !l.startsWith(">"));
  return ATTACHMENT_WORDS.test(`${d.subject}\n${own.join("\n")}`);
}

const MAIL_FONT = "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif";
/** HTML du message tel qu'il part : styles en ligne (les clients mail ignorent les feuilles de style). */
export function emailHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("p,h2,h3,ul,ol").forEach((n) => ((n as HTMLElement).style.margin = "0 0 12px"));
  doc.querySelectorAll("blockquote").forEach((n) => ((n as HTMLElement).style.cssText = "margin:0 0 12px 4px;padding-left:12px;border-left:2px solid #c9c9d1;color:#555"));
  return `<div style="font-family:${MAIL_FONT};font-size:14px;line-height:1.55">${doc.body.innerHTML}</div>`;
}

const NEUTRAL_KEYWORDS = new Set(["inherit", "initial", "unset", "currentcolor", "windowtext", "black", "white", "gray", "grey", "silver", "dimgray", "dimgrey", "darkgray", "darkgrey", "lightgray", "lightgrey", "gainsboro", "whitesmoke"]);

/** Noir, blanc ou gris quasi pur : la couleur de texte par défaut du document d'origine (collé,
 *  cité), pas un choix de mise en forme. Valeur telle que la renvoie `element.style.color`
 *  (`rgb()` ou mot-clé). Écart toléré entre composantes : 12, sous le gris bleuté de la palette
 *  de l'éditeur (#6b6b78), qui reste donc une vraie couleur. */
export function isNeutralColor(value: string): boolean {
  const v = value.trim().toLowerCase();
  if (NEUTRAL_KEYWORDS.has(v)) return true;
  const rgb = /^rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(v)?.slice(1).map(Number);
  return !!rgb && Math.max(...rgb) - Math.min(...rgb) <= 12;
}

const bodyHtmlOf = (m: DetailMessage) => (m.bodyHtml?.trim() ? sanitizeFragment(m.bodyHtml) : m.bodyText?.trim() ? textToHtml(m.bodyText) : "");

const who = (name: string | null | undefined, address: string) => (name ? `${name} <${address}>` : address);
const when = (iso: string) => new Date(iso).toLocaleString("fr-FR", { dateStyle: "full", timeStyle: "short" });
const sameAddr = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const toRecipient = (a: Addr): Recipient => ({ address: a.address, name: a.name });

/** Heures proposées par « Envoyer plus tard » (le lundi n'est pas répété quand demain en est un). */
export function scheduleOptions(now: Date): { label: string; at: Date }[] {
  const at = (days: number, hour: number) => { const d = new Date(now); d.setDate(d.getDate() + days); d.setHours(hour, 0, 0, 0); return d; };
  const fmt = (d: Date) => d.toLocaleString("fr-FR", { weekday: "long", hour: "numeric", minute: "2-digit" });
  const out = [{ label: "Dans 1 heure", at: new Date(now.getTime() + 3600_000) }, { label: `Demain matin — ${fmt(at(1, 8))}`, at: at(1, 8) }];
  const toMonday = (8 - now.getDay()) % 7 || 7;
  if (toMonday > 1) out.push({ label: `Lundi matin — ${fmt(at(toMonday, 8))}`, at: at(toMonday, 8) });
  return out;
}

/** Délais proposés par « Me relancer si pas de réponse » (à 8 h). */
export function followupOptions(now: Date): { label: string; at: Date }[] {
  return [3, 7, 14].map((days) => {
    const at = new Date(now);
    at.setDate(at.getDate() + days);
    at.setHours(8, 0, 0, 0);
    return { label: days === 7 ? "Dans 1 semaine" : `Dans ${days} jours`, at };
  });
}

/** « Nom <adresse> », ou l'adresse seule. */
export const senderLabel = (name: string | null | undefined, email: string) => (name ? `${name} <${email}>` : email);

/** Expéditeurs possibles : chaque compte puis ses alias (`alias` : adresse de l'alias, vide pour le compte). */
export function senderOptions(accounts: Account[]) {
  return accounts.flatMap((a) => [
    { key: `${a.id}|`, accountId: a.id, alias: "", label: senderLabel(a.displayName, a.email) },
    ...a.identities.map((i) => ({ key: `${a.id}|${i.email}`, accountId: a.id, alias: i.email, label: senderLabel(i.name, i.email) })),
  ]);
}

/** L'alias de `fromEmail` s'il existe encore sur ce compte ; sinon rien (le compte lui-même écrit). */
export const knownAlias = (account: Account | undefined, fromEmail: string | undefined) =>
  account?.identities.find((i) => i.email.toLowerCase() === fromEmail?.toLowerCase())?.email;

/** Brouillon de réponse / transfert à partir de la conversation. `own` : adresse du compte ; `aliases` : ses autres adresses d'envoi. */
export function draftFrom(mode: Exclude<ComposeMode, "new">, thread: ThreadDetail, own: string, signature = "", aliases: string[] = []): Draft {
  const isMine = (address: string) => [own, ...aliases].some((m) => sameAddr(m, address));
  const msgs = thread.messages;
  // On répond au dernier message reçu ; si tous sont de nous, au dernier.
  const target = [...msgs].reverse().find((m) => !isMine(m.fromAddr)) ?? msgs[msgs.length - 1];
  // On répond depuis l'adresse à laquelle le message a été envoyé, si c'est un alias.
  const fromEmail = aliases.find((alias) => [...target.to, ...target.cc].some((a) => sameAddr(a.address, alias)));
  const base = { ...emptyDraft(thread.accountId), fromEmail };
  const header = `Le ${when(target.date)}, ${who(target.fromName, target.fromAddr)} a écrit :`;

  if (mode === "forward") {
    const lines = [
      "---------- Message transféré ----------",
      `De : ${who(target.fromName, target.fromAddr)}`,
      `Date : ${when(target.date)}`,
      `Objet : ${target.subject}`,
      `À : ${target.to.map((a) => who(a.name, a.address)).join(", ")}`,
    ];
    const head = lines.map((l) => `<p>${escapeHtml(l)}</p>`).join("");
    return { ...base, mode, subject: withPrefix("Fwd", thread.subject), html: `<p></p>${signatureHtml(signature)}${head}${bodyHtmlOf(target)}` };
  }

  const fromMe = isMine(target.fromAddr);
  const primary: Recipient[] = fromMe ? target.to.map(toRecipient) : [{ address: target.fromAddr, name: target.fromName }];
  let cc: Recipient[] = [];
  if (mode === "replyAll") {
    const seen = new Set([own, ...aliases, ...primary.map((r) => r.address)].map((m) => m.toLowerCase()));
    cc = [...target.to, ...target.cc].filter((a) => !seen.has(a.address.toLowerCase()) && seen.add(a.address.toLowerCase())).map(toRecipient);
  }
  const refs = [...target.references, ...(target.messageId ? [target.messageId] : [])];
  return {
    ...base,
    mode,
    to: primary,
    cc,
    subject: withPrefix("Re", thread.subject),
    html: `<p></p>${signatureHtml(signature)}<p>${escapeHtml(header)}</p><blockquote>${bodyHtmlOf(target)}</blockquote>`,
    inReplyTo: target.messageId ?? undefined,
    references: [...new Set(refs)],
  };
}
