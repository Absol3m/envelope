import type { Addr, DetailMessage, OutgoingMessage, ThreadDetail } from "./api";

export type ComposeMode = "new" | "reply" | "replyAll" | "forward";

export interface Recipient {
  address: string;
  name?: string | null;
}
export interface Draft {
  /** Identifie le brouillon : l'éditeur est recréé quand il change. */
  id: string;
  accountId: string;
  mode: ComposeMode;
  to: Recipient[];
  cc: Recipient[];
  bcc: Recipient[];
  subject: string;
  /** Corps au format HTML produit par l'éditeur riche. */
  html: string;
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
  };
}

export const isBlank = (d: Draft) => !d.to.length && !d.cc.length && !d.bcc.length && !d.subject.trim() && !htmlToPlain(d.html);

export function emptyDraft(accountId: string): Draft {
  return { id: crypto.randomUUID(), accountId, mode: "new", to: [], cc: [], bcc: [], subject: "", html: "", references: [] };
}

const PREFIX = /^\s*((re|tr|fwd?|fw)\s*:\s*)+/i;
export const withPrefix = (prefix: "Re" | "Fwd", subject: string) => `${prefix}: ${subject.replace(PREFIX, "").trim()}`.trim();

const escapeHtml = (t: string) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** Texte brut → paragraphes HTML (lignes vides = nouveaux paragraphes). */
export function textToHtml(text: string): string {
  return text
    .trim()
    .split(/\r?\n{2,}/)
    .map((p) => `<p>${escapeHtml(p).replace(/\r?\n/g, "<br>")}</p>`)
    .join("");
}

/** HTML reçu → fragment sûr à citer : sans script, style ni image (les balises inconnues sont de toute façon écartées par l'éditeur). */
function quotable(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("script,style,head,img,iframe,object,embed,link,meta,form").forEach((n) => n.remove());
  return doc.body.innerHTML;
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
      case "SCRIPT": case "STYLE": return "";
      default: return inner();
    }
  };
  return render(doc.body).replace(/\n{3,}/g, "\n\n").trim();
}

const MAIL_FONT = "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif";
/** HTML du message tel qu'il part : styles en ligne (les clients mail ignorent les feuilles de style). */
export function emailHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("p,h2,h3").forEach((n) => ((n as HTMLElement).style.margin = "0 0 12px"));
  doc.querySelectorAll("ul,ol").forEach((n) => ((n as HTMLElement).style.margin = "0 0 12px"));
  doc.querySelectorAll("blockquote").forEach((n) => ((n as HTMLElement).style.cssText = "margin:0 0 12px 4px;padding-left:12px;border-left:2px solid #c9c9d1;color:#555"));
  return `<div style="font-family:${MAIL_FONT};font-size:14px;line-height:1.55">${doc.body.innerHTML}</div>`;
}

const bodyHtmlOf = (m: DetailMessage) => (m.bodyHtml?.trim() ? quotable(m.bodyHtml) : m.bodyText?.trim() ? textToHtml(m.bodyText) : "");

const who = (name: string | null | undefined, address: string) => (name ? `${name} <${address}>` : address);
const when = (iso: string) => new Date(iso).toLocaleString("fr-FR", { dateStyle: "full", timeStyle: "short" });
const sameAddr = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const toRecipient = (a: Addr): Recipient => ({ address: a.address, name: a.name });

/** Brouillon de réponse / transfert à partir de la conversation. `own` : adresse du compte. */
export function draftFrom(mode: Exclude<ComposeMode, "new">, thread: ThreadDetail, own: string): Draft {
  const msgs = thread.messages;
  // On répond au dernier message reçu ; si tous sont de nous, au dernier.
  const target = [...msgs].reverse().find((m) => !sameAddr(m.fromAddr, own)) ?? msgs[msgs.length - 1];
  const base = emptyDraft(thread.accountId);
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
    return { ...base, mode, subject: withPrefix("Fwd", thread.subject), html: `<p></p>${head}${bodyHtmlOf(target)}` };
  }

  const fromMe = sameAddr(target.fromAddr, own);
  const primary: Recipient[] = fromMe ? target.to.map(toRecipient) : [{ address: target.fromAddr, name: target.fromName }];
  let cc: Recipient[] = [];
  if (mode === "replyAll") {
    const seen = new Set([own.toLowerCase(), ...primary.map((r) => r.address.toLowerCase())]);
    cc = [...target.to, ...target.cc].filter((a) => !seen.has(a.address.toLowerCase()) && seen.add(a.address.toLowerCase())).map(toRecipient);
  }
  const refs = [...target.references, ...(target.messageId ? [target.messageId] : [])];
  return {
    ...base,
    mode,
    to: primary,
    cc,
    subject: withPrefix("Re", thread.subject),
    html: `<p></p><p>${escapeHtml(header)}</p><blockquote>${bodyHtmlOf(target)}</blockquote>`,
    inReplyTo: target.messageId ?? undefined,
    references: [...new Set(refs)],
  };
}
