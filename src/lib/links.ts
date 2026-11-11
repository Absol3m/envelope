import { useMail } from "./store";
import { openExternal } from "./utils";

/* Liens des e-mails : détection des destinations trompeuses (hameçonnage), avant ouverture dans le navigateur.
   Analyse d'URL par le moteur (`URL`), pas d'expression maison ; seul le texte affiché est repéré par un motif simple. */

/** Extensions de fichiers courantes qui sont aussi des suffixes d'adresse possibles : un texte « rapport.pdf » n'annonce pas un site. */
const FILE_EXTENSIONS = new Set(
  "pdf doc docx xls xlsx ppt pptx txt csv zip rar png jpg jpeg gif svg mp3 mp4 mov ics eml json xml html htm js css exe dmg msi rtf odt ods py sh rs".split(" "),
);

/** Hôte annoncé par le texte du lien s'il ressemble à une adresse web (« www.banque.fr », « https://banque.fr/x »). */
function shownHost(text: string): string | null {
  const t = text.trim();
  if (!t || /\s/.test(t) || t.length > 200) return null;
  try {
    const u = new URL(/^[a-z][a-z0-9+.-]*:/i.test(t) ? t : `https://${t}`);
    if (!/^https?:$/.test(u.protocol) || !u.hostname.includes(".")) return null;
    const host = u.hostname.toLowerCase();
    if (!/\.[a-z]{2,}$/.test(host)) return null; // « v1.2 », « 12.5 »
    // Un nom de fichier (« rapport.pdf ») n'est pas une adresse : seul un texte nu se termine ainsi.
    const bare = !/^(https?:\/\/|www\.)/i.test(t);
    return bare && FILE_EXTENSIONS.has(host.slice(host.lastIndexOf(".") + 1)) ? null : host;
  } catch {
    return null;
  }
}

/** Deux hôtes désignent le même site : identiques, ou l'un est un sous-domaine de l'autre (« www.x.fr » / « x.fr »). */
const sameSite = (a: string, b: string) => a === b || a.endsWith(`.${b}`) || b.endsWith(`.${a}`);

/** Phrase expliquant pourquoi la destination d'un lien est douteuse ; `null` si rien à signaler. */
export function linkRisk(href: string, text: string): string | null {
  let u: URL;
  try {
    u = new URL(href);
  } catch {
    return "Ce lien n'est pas une adresse web ordinaire.";
  }
  if (u.protocol === "mailto:") return null;
  if (u.protocol !== "http:" && u.protocol !== "https:") return "Ce lien n'est pas une adresse web ordinaire.";
  const host = u.hostname.toLowerCase();
  if (u.username || u.password) return `Ce lien contient un identifiant avant le nom du site ; il mène en réalité à « ${host} ».`;
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(host) || host.startsWith("[")) return `Ce lien mène à une adresse numérique (« ${host} »), pas à un nom de site.`;
  // Les IDN s'écrivent « xn-- » une fois normalisés : domaine pouvant imiter un autre par des lettres d'autres alphabets.
  if (host.split(".").some((l) => l.startsWith("xn--"))) return `Le site « ${host} » utilise un nom en caractères non latins qui peut imiter un autre site.`;
  const shown = shownHost(text);
  return shown && !sameSite(shown, host) ? `Le texte du lien annonce « ${shown} », mais il mène à « ${host} ».` : null;
}

/** Ouvre un lien dans le navigateur système ; si sa destination est douteuse, demande d'abord confirmation
 *  (boîte de dialogue native). `text` : texte affiché du lien. */
export async function openLink(href: string, text: string): Promise<void> {
  const risk = linkRisk(href, text);
  if (risk) {
    const shown = href.length > 160 ? `${href.slice(0, 160)}…` : href;
    const ok = await useMail.getState().backend.confirm(`${risk}\n\n${shown}\n\nCe peut être une tentative d'hameçonnage. L'ouvrir quand même ?`, "Ouvrir quand même");
    if (!ok) return;
  }
  await openExternal(href);
}
