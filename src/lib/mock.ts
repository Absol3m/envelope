import { DEMO_EN, DEMO_EN_HTML } from "./mock.en";
import { i18next, locale, t } from "./i18n";
import type { Account, Backend, ReminderKind, Rule, Detected, SendReport, DiskEncryption, LockStatus, OutgoingMessage, Template, DetailAttachment, DetailMessage, FolderRow, McpInfo, ThreadDetail, ThreadQuery, ThreadSummary } from "./api";

/* Boîte de démonstration : données crédibles, en mémoire. Sert à présenter l'interface sans
   compte (et en navigateur, hors Tauri). Les actions (lire, suivre, archiver) y sont réelles. */

const NOW = Date.now();
const ago = (minutes: number) => new Date(NOW - minutes * 60_000).toISOString();

/** Verrouillage de la démonstration (mot de passe : demo1234). */
const LOCK: LockStatus = { method: "off", locked: false, timeoutSecs: 0, systemAvailable: true };

const QUEUE: { id: number; accountId: string; message: OutgoingMessage; sendAt: number }[] = [];

const FR_TEMPLATE_BODY = "<p>Bonjour,</p><p>Nous avons bien reçu votre message et revenons vers vous sous 48 h.</p><p>Cordialement,</p>";
const TEMPLATES: Template[] = [{ id: 1, name: "Accusé de réception", subject: "Re : votre demande", bodyHtml: FR_TEMPLATE_BODY }];

const ACCOUNTS: Account[] = [
  { id: "pro", email: "alex@atelier-nord.fr", displayName: "Alex Morel", provider: "imap", authKind: "password", syncStatus: "idle", syncError: null, lastSyncAt: ago(2), signature: "Alex Morel\nAtelier Nord", identities: [{ id: 1, name: "Atelier Nord", email: "contact@atelier-nord.fr" }] },
  { id: "perso", email: "alex.morel@example.com", displayName: "Alex Morel", provider: "gmail", authKind: "oauth2", syncStatus: "idle", syncError: null, lastSyncAt: ago(2), signature: "", identities: [] },
];

const ROLES = ["inbox", "drafts", "sent", "archive", "junk", "trash"] as const;
const roleName = (role: string) => t(`nav:views.${role}`);
const folderId = (acct: string, role: string) => (acct === "pro" ? 1 : 101) + ROLES.indexOf(role as (typeof ROLES)[number]);

interface MockThread {
  id: number;
  accountId: string;
  role: (typeof ROLES)[number];
  starred: boolean;
  messages: DetailMessage[];
  attachments: DetailAttachment[];
  reminder?: { kind: ReminderKind; due: number; fired?: number };
}

const RULES: Rule[] = [{ id: 1, name: "Newsletters", enabled: true, accountId: null, query: "de:newsletter", markRead: true, star: false, moveTo: "archive" }];

let nextMsg = 1;
let nextAtt = 1;
const me = (acct: string) => ({ name: "Alex Morel", address: ACCOUNTS.find((a) => a.id === acct)!.email });

/** Le jeu de données est écrit en français ; en anglais, chaque texte est remplacé par sa version de `mock.en.ts`. */
const english = () => locale() === "en";
const tx = (text: string) => (english() ? DEMO_EN[text] ?? text : text);
const htmlCache = new Map<string, string>();
const txHtml = (html: string) => {
  if (!english()) return html;
  let out = htmlCache.get(html);
  if (out === undefined) htmlCache.set(html, (out = DEMO_EN_HTML.reduce((h, [fr, en]) => h.split(fr).join(en), html)));
  return out;
};

function msg(p: { from: [string | null, string]; to?: [string | null, string][]; cc?: [string | null, string][]; at: number; text?: string; html?: string; read?: boolean; subject: string; folder?: number; unsubscribe?: DetailMessage["unsubscribe"]; authFail?: DetailMessage["authFail"] }): DetailMessage {
  return {
    id: nextMsg++,
    messageId: `demo-${nextMsg}@envelope.local`,
    references: [],
    folderId: p.folder ?? 1,
    subject: tx(p.subject),
    fromName: p.from[0],
    fromAddr: p.from[1],
    to: (p.to ?? []).map(([name, address]) => ({ name, address })),
    cc: (p.cc ?? []).map(([name, address]) => ({ name, address })),
    date: ago(p.at),
    bodyText: p.text ? tx(p.text) : null,
    bodyHtml: p.html ? txHtml(p.html) : null,
    isRead: p.read ?? true,
    isStarred: false,
    unsubscribe: p.unsubscribe ?? null,
    authFail: p.authFail ?? null,
  };
}
const att = (messageId: number, filename: string, mimeType: string, size: number): DetailAttachment => ({
  id: nextAtt++, messageId, filename, mimeType, size, contentId: null, isInline: false,
});

const svg = (a: string, b: string, label = "") =>
  `data:image/svg+xml;utf8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="600" height="220"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient></defs><rect width="600" height="220" fill="url(#g)"/><text x="32" y="124" font-family="Helvetica,Arial" font-size="34" font-weight="700" fill="white">${label}</text></svg>`)}`;

const me1 = () => me("pro");
const T: MockThread[] = [];
const add = (accountId: string, role: MockThread["role"], messages: DetailMessage[], attachments: DetailAttachment[] = [], starred = false) => {
  const id = T.length + 1;
  messages.forEach((m) => (m.folderId = folderId(accountId, role)));
  T.push({ id, accountId, role, starred, messages, attachments });
};
const proMe: [string | null, string] = ["Alex Morel", "alex@atelier-nord.fr"];
const persoMe: [string | null, string] = ["Alex Morel", "alex.morel@example.com"];

/** (Re)construit les conversations de démonstration dans la langue active. */
function seed() {
  T.length = 0;
  nextMsg = 1;
  nextAtt = 1;
  // 1 — Revue de roadmap (pro, non lu, pièce jointe)
  {
    const m1 = msg({ subject: "Revue de la roadmap T1 — besoin de votre avis", from: ["Camille Laurent", "camille.laurent@atelier-nord.fr"], to: [proMe], at: 190, text: "Bonjour Alex,\n\nOn fige la roadmap du T1 vendredi. Avant ça, j'aimerais ton retour sur trois points :\n\n1. Le calendrier de la refonte du client mail (sortie bêta fin février ?)\n2. L'arbitrage entre synchronisation hors-ligne et recherche plein texte\n3. La charge côté design pour les raccourcis clavier\n\nJe t'ai joint la version 3 du document.\n\nMerci d'avance,\nCamille" });
    const m2 = msg({ subject: "Re: Revue de la roadmap T1 — besoin de votre avis", from: ["Alex Morel", "alex@atelier-nord.fr"], to: [["Camille Laurent", "camille.laurent@atelier-nord.fr"]], at: 120, text: "Salut Camille,\n\nJe regarde ça cet après-midi. Sur le point 2, je pense qu'on peut tout livrer : la recherche est déjà indexée en local (FTS5), la synchro hors-ligne est le vrai sujet.\n\nJe te fais un retour détaillé avant 17 h.\n\nAlex\n\n> Bonjour Alex,\n> On fige la roadmap du T1 vendredi." });
    const m3 = msg({ subject: "Re: Revue de la roadmap T1 — besoin de votre avis", from: ["Camille Laurent", "camille.laurent@atelier-nord.fr"], to: [proMe], cc: [["Thomas Bernard", "thomas.bernard@atelier-nord.fr"]], at: 22, read: false, text: "Parfait, merci !\n\nThomas est en copie : il pourra relire la partie technique en parallèle. Si tu peux aussi ajouter une estimation en jours pour le serveur MCP, ça m'aiderait pour la présentation de jeudi.\n\nCamille\n\n> Je te fais un retour détaillé avant 17 h.\n> Alex" });
    add("pro", "inbox", [m1, m2, m3], [att(m1.id, "Roadmap-T1-v3.pdf", "application/pdf", 1_240_000)]);
  }
  // 2 — Devis (pro, suivi, 2 PJ)
  {
    const m1 = msg({ subject: "Devis refonte site — version finale", from: ["Thomas Bernard", "thomas.bernard@atelier-nord.fr"], to: [proMe], at: 1700, text: "Alex, voici la version finale du devis pour la refonte. J'ai intégré tes remarques sur la maintenance.\n\nThomas" });
    const m2 = msg({ subject: "Re: Devis refonte site — version finale", from: ["Alex Morel", "alex@atelier-nord.fr"], to: [["Thomas Bernard", "thomas.bernard@atelier-nord.fr"]], at: 1500, text: "Merci Thomas. Une question : le forfait de maintenance inclut-il les évolutions mineures ?" });
    const m3 = msg({ subject: "Re: Devis refonte site — version finale", from: ["Thomas Bernard", "thomas.bernard@atelier-nord.fr"], to: [proMe], at: 400, text: "Oui, jusqu'à 4 heures par mois. Au-delà, c'est facturé au taux horaire du devis.\n\n> Une question : le forfait de maintenance inclut-il les évolutions mineures ?" });
    const m4 = msg({ subject: "Re: Devis refonte site — version finale", from: ["Thomas Bernard", "thomas.bernard@atelier-nord.fr"], to: [proMe], at: 118, read: false, text: "Dernier point : le client signe demain matin. Je t'envoie aussi les CGV mises à jour, à joindre au bon de commande.\n\nThomas" });
    add("pro", "inbox", [m1, m2, m3, m4], [att(m1.id, "Devis-Refonte-2026.pdf", "application/pdf", 482_000), att(m4.id, "CGV-2026.pdf", "application/pdf", 214_000)], true);
  }
  // 3 — Facture (pro, non lu)
  add("pro", "inbox", [msg({ subject: "Question sur la facture #2026-0142", from: ["Sophie Martin", "sophie.martin@cabinet-martin.fr"], to: [proMe], at: 300, read: false, text: "Bonjour,\n\nNous avons bien reçu la facture #2026-0142 mais le montant HT ne correspond pas au devis signé (2 400 € au lieu de 2 150 €).\n\nPourriez-vous la vérifier et nous renvoyer une version corrigée ?\n\nCordialement,\nSophie Martin\nCabinet Martin & Associés" })]);
  // 4 — GitHub (pro, notification, HTML)
  add("pro", "inbox", [msg({
    subject: "[envelope] Pull request #14 : Ajout du serveur MCP en lecture seule", from: ["GitHub", "notifications@github.com"], to: [proMe], at: 40, read: false,
    html: `<div style="font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;color:#1f2328;font-size:14px;line-height:1.5"><p><strong>Absol3m</strong> a ouvert une pull request :</p><div style="border:1px solid #d0d7de;border-radius:8px;padding:14px 16px;margin:12px 0"><div style="font-size:16px;font-weight:600">Ajout du serveur MCP en lecture seule <span style="color:#6e7781;font-weight:400">#14</span></div><div style="color:#57606a;margin-top:4px">main ← feature/mcp-readonly · <span style="color:#1a7f37">+1 482</span> <span style="color:#cf222e">−37</span></div></div><p>Cette PR introduit <code style="background:#eff1f3;padding:2px 6px;border-radius:6px">envelope-mcp</code> :</p><ul><li>JSON-RPC 2.0 sur stdio</li><li>5 outils, tous en <strong>lecture seule</strong></li><li>Avertissement systématique sur le contenu non fiable</li></ul><pre style="background:#f6f8fa;border-radius:8px;padding:12px;overflow:auto;font-size:12px">claude mcp add envelope -- ./envelope-mcp</pre><p><a href="https://github.com/Absol3m/envelope/pull/14" style="display:inline-block;background:#1f883d;color:#fff;text-decoration:none;padding:8px 16px;border-radius:6px;font-weight:600">Voir la pull request</a></p><p style="color:#6e7781;font-size:12px;margin-top:24px">Vous recevez ceci car vous suivez ce dépôt.</p></div>`,
  })]);
  // 5 — Calendrier (pro)
  add("pro", "inbox", [msg({ subject: "Invitation : Point hebdo équipe — lun. 10:00", from: ["Google Agenda", "calendar-notification@google.com"], to: [proMe], at: 180, read: false, text: "Point hebdo équipe\nlundi 10:00 – 10:45 (heure de Paris)\nLieu : Salle Gutenberg / Google Meet\n\nParticipants : Camille Laurent, Thomas Bernard, Julie Moreau, Alex Morel\n\nRépondre : Oui · Non · Peut-être" })]);
  // 6 — Stripe (pro, HTML, lu)
  add("pro", "inbox", [msg({
    subject: "Reçu de paiement — 49,00 €", from: ["Stripe", "receipts@stripe.com"], to: [proMe], at: 1620,
    html: `<div style="font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;max-width:480px;margin:0 auto;color:#32325d"><h2 style="margin:0 0 4px">Reçu de paiement</h2><p style="color:#8898aa;margin:0 0 20px">Payé le 8 novembre 2026</p><table style="width:100%;border-collapse:collapse;font-size:14px"><tr><td style="padding:10px 0;border-bottom:1px solid #e6ebf1">Abonnement Atelier Pro (mensuel)</td><td style="text-align:right;border-bottom:1px solid #e6ebf1">49,00 €</td></tr><tr><td style="padding:10px 0;border-bottom:1px solid #e6ebf1;color:#8898aa">TVA (20 %)</td><td style="text-align:right;border-bottom:1px solid #e6ebf1;color:#8898aa">inclus</td></tr><tr><td style="padding:12px 0;font-weight:700">Total</td><td style="text-align:right;font-weight:700">49,00 €</td></tr></table><p style="color:#8898aa;font-size:12px;margin-top:24px">Carte Visa se terminant par 4242 · Reçu n° 2184-9031</p></div>`,
  })]);
  // 7 — Newsletter du matin (pro, HTML avec image distante → bannière « images bloquées »)
  add("pro", "inbox", [msg({
    subject: "La Lettre du matin : l'IA dans la boîte mail, vraiment utile ?", from: ["La Lettre Tech", "newsletter@lalettretech.fr"], to: [proMe], at: 55, read: false, unsubscribe: { url: "https://lalettretech.fr/desabo/42", mailto: null, oneClick: true },
    html: `<div style="font-family:Georgia,'Times New Roman',serif;max-width:560px;margin:0 auto;color:#222"><img src="https://images.lalettretech.fr/hero/2026-11-09.jpg" alt="" width="560" style="width:100%;border-radius:10px;background:#eee;min-height:120px"/><img src="${svg("#6d5dfc","#2fb1e8","MCP")}" alt="MCP" width="560" style="width:100%;border-radius:10px;margin-top:16px"/><h1 style="font-size:26px;line-height:1.2;margin:20px 0 8px">L'IA dans la boîte mail : gadget ou vraie révolution ?</h1><p style="color:#666;margin:0 0 18px">Lundi 9 novembre · 6 min de lecture</p><p style="font-size:16px;line-height:1.6">Résumés automatiques, réponses suggérées, tri intelligent… Les clients mail sont devenus le premier terrain d'expérimentation des agents. Mais qui contrôle vraiment ce que l'agent lit — et envoie — en votre nom ?</p><p style="font-size:16px;line-height:1.6">Dans cette édition : le protocole <strong>MCP</strong> expliqué simplement, trois outils open source à essayer cette semaine, et pourquoi le contenu d'un e-mail doit toujours être traité comme <em>non fiable</em>.</p><p><a href="https://lalettretech.fr/edition/212" style="display:inline-block;background:#111;color:#fff;padding:12px 22px;border-radius:999px;text-decoration:none;font-family:Helvetica,Arial,sans-serif;font-size:14px">Lire l'édition complète →</a></p><p style="color:#999;font-size:12px;font-family:Helvetica,Arial,sans-serif;margin-top:32px">Vous recevez cet e-mail car vous êtes abonné. <a href="https://lalettretech.fr/unsub" style="color:#999">Se désabonner</a></p></div>`,
  })]);
  // 7b — Faux message de banque (usurpation : DMARC en échec, lien dont le texte annonce un autre site, pixel espion)
  add("perso", "inbox", [msg({
    subject: "Action requise : votre compte sera suspendu", from: ["Service Sécurité", "securite@banque-exemple.fr"], to: [persoMe], at: 70, read: false, authFail: "dmarc",
    html: `<div style="font-family:Helvetica,Arial,sans-serif;max-width:480px;margin:0 auto;color:#222"><p>Bonjour,</p><p>Une activité inhabituelle a été détectée sur votre compte. Vérifiez votre identité dans les 24 h : <a href="https://verification-compte.example.net/login">www.banque-exemple.fr/verification</a></p><img src="https://pixel.example.net/o.gif" width="1" height="1" alt=""/></div>`,
  })]);
  // 8 — Hebdo Rust (pro, newsletter, HTML sans image distante)
  add("pro", "inbox", [msg({
    subject: "Hebdo Rust France #212 — async fn dans les traits, sqlx 0.9, Tauri 3 alpha", from: ["Hebdo Rust France", "hello@rust-france.org"], to: [proMe], at: 600, text: "Cette semaine :\n\n• async fn dans les traits : retour d'expérience après un an\n• sqlx 0.9 : ce qui change\n• Tauri 3 alpha : migration depuis la v2\n• Un client mail en Rust, de zéro à la bêta\n\nBonne lecture !\nL'équipe Rust France",
  })]);
  // 9 — RH (pro, lu)
  add("pro", "inbox", [msg({ subject: "Votre entretien de vendredi", from: ["Julie Moreau", "julie.moreau@atelier-nord.fr"], to: [proMe], at: 2900, text: "Bonjour Alex,\n\nJe confirme notre entretien annuel vendredi à 14 h 30, salle Gutenberg. Pensez à préparer un bilan de vos projets de l'année : l'équipe attend surtout votre point de vue sur le client mail.\n\nÀ vendredi,\nJulie" })]);
  // 10 — Alerte sécurité (pro)
  add("pro", "inbox", [msg({ subject: "Alerte sécurité : nouvelle connexion à votre compte", from: ["Sécurité Atelier Nord", "no-reply@atelier-nord.fr"], to: [proMe], at: 4300, text: "Une nouvelle connexion a été détectée :\n\nAppareil : MacBook Pro (macOS)\nLieu approximatif : Lille, France\nDate : 6 novembre, 21:14\n\nSi c'était vous, aucune action n'est nécessaire." })]);
  // 11 — Maman (perso, lu)
  add("perso", "inbox", [
    msg({ subject: "Dimanche chez nous ?", from: ["Maman", "maman@example.org"], to: [persoMe], at: 1900, text: "Coucou mon grand,\n\nOn fait un gratin dimanche midi, tu viens avec Léa ? Papa a ressorti les vieilles photos, il veut absolument te les montrer.\n\nBisous" }),
    msg({ subject: "Re: Dimanche chez nous ?", from: ["Alex Morel", "alex.morel@example.com"], to: [["Maman", "maman@example.org"]], at: 1850, text: "Avec plaisir ! On arrive vers midi. On apporte le dessert.\n\n> On fait un gratin dimanche midi, tu viens avec Léa ?" }),
  ]);
  // 12 — Photos (perso, PJ image, non lu)
  {
    const m = msg({ subject: "Photos du week-end", from: ["Lucas Petit", "lucas.petit@gmail.com"], to: [persoMe], at: 270, read: false, text: "Salut ! Les photos de la rando, comme promis. La dernière est ma préférée, on dirait une carte postale.\n\nOn remet ça au printemps ?" });
    add("perso", "inbox", [m], [att(m.id, "sommet-vosges.jpg", "image/jpeg", 3_450_000), att(m.id, "pique-nique.jpg", "image/jpeg", 2_870_000)]);
  }
  // 13 — Voyage (perso, suivi)
  {
    const m = msg({ subject: "Votre réservation à Lisbonne est confirmée", from: ["Airbnb", "automated@airbnb.com"], to: [persoMe], at: 4800, text: "Bonne nouvelle, c'est confirmé !\n\nAppartement lumineux à Alfama\n12 → 17 décembre 2026 · 2 voyageurs\nTotal : 612,40 €\n\nVotre hôte, Marta, vous contactera 48 h avant l'arrivée." });
    add("perso", "inbox", [m], [att(m.id, "Confirmation-Lisbonne.pdf", "application/pdf", 156_000)], true);
  }
  // 14 — Banque (perso, notif)
  add("perso", "inbox", [msg({ subject: "Votre relevé de novembre est disponible", from: ["Crédit Mutuel", "ne-pas-repondre@creditmutuel.fr"], to: [persoMe], at: 7200, text: "Votre relevé de compte de novembre est disponible dans votre espace client.\n\nPour votre sécurité, ne communiquez jamais vos identifiants par e-mail." })]);
  // 15 — Figma (perso)
  add("perso", "inbox", [msg({ subject: "Camille a commenté « Design System v2 »", from: ["Figma", "no-reply@figma.com"], to: [persoMe], at: 95, read: false, text: "Camille Laurent a laissé un commentaire sur « Design System v2 » :\n\n« Le contraste du texte secondaire est un peu faible en mode sombre, on peut l'éclaircir de 5 % ? »\n\nRépondre dans Figma" })]);
  // Archive / envoyés pour peupler les dossiers
  add("pro", "archive", [msg({ subject: "Compte rendu — atelier design du 3 novembre", from: ["Camille Laurent", "camille.laurent@atelier-nord.fr"], to: [proMe], at: 9800, text: "Voici le compte rendu de l'atelier. Les décisions sont en gras dans le document partagé." })]);
  add("pro", "sent", [msg({ subject: "Re: Planning de décembre", from: ["Alex Morel", "alex@atelier-nord.fr"], to: [["Julie Moreau", "julie.moreau@atelier-nord.fr"]], at: 3100, text: "Merci Julie, c'est noté pour la semaine du 14." })]);
  void me1;
  TEMPLATES[0] = { ...TEMPLATES[0], name: tx("Accusé de réception"), subject: tx("Re : votre demande"), bodyHtml: txHtml(FR_TEMPLATE_BODY) };
}
seed();
i18next.on("languageChanged", seed);

// ───────────── Implémentation du Backend ─────────────

const delay = <V>(v: V, ms = 140) => new Promise<V>((r) => setTimeout(() => r(v), ms + Math.random() * 120));

const lastMessage = (t: MockThread) => t.messages[t.messages.length - 1];
function summarize(t: MockThread): ThreadSummary {
  const last = lastMessage(t);
  const names: string[] = [];
  for (const m of t.messages) {
    const n = m.fromName ?? m.fromAddr;
    if (!names.includes(n)) names.push(n);
  }
  return {
    id: t.id,
    accountId: t.accountId,
    subject: t.messages[0].subject.replace(/^(re|fwd?|tr):\s*/i, ""),
    snippet: (last.bodyText ?? htmlToText(last.bodyHtml ?? "")).split(/\n>/)[0].replace(/\s+/g, " ").trim().slice(0, 160),
    lastMessageAt: last.date,
    messageCount: t.messages.length,
    unreadCount: t.messages.filter((m) => !m.isRead).length,
    isStarred: t.starred,
    hasAttachments: t.attachments.length > 0,
    fromName: last.fromName,
    fromAddr: last.fromAddr,
    participants: names.slice(0, 4).join(", "),
    folderId: folderId(t.accountId, t.role),
    reminderKind: t.reminder?.kind ?? null,
    reminderDue: t.reminder?.due ?? null,
    reminderFired: t.reminder?.fired ?? null,
  };
}
const htmlToText = (html: string) => html.replace(/<style[\s\S]*?<\/style>/gi, "").replace(/<[^>]+>/g, " ");

const folders = (): FolderRow[] =>
  ACCOUNTS.flatMap((a) =>
    ROLES.map((role) => {
      const inRole = T.filter((t) => t.accountId === a.id && t.role === role);
      return {
        id: folderId(a.id, role), accountId: a.id, path: role.toUpperCase(), name: roleName(role), role,
        unreadCount: inRole.reduce((n, t) => n + t.messages.filter((m) => !m.isRead).length, 0),
        totalCount: inRole.reduce((n, t) => n + t.messages.length, 0),
      };
    }),
  );

const norm = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();

function listThreads(q: ThreadQuery): ThreadSummary[] {
  const words = q.query ? norm(q.query).split(/\s+/).filter(Boolean) : [];
  return T.filter((t) => {
    if (q.folderId !== undefined && folderId(t.accountId, t.role) !== q.folderId) return false;
    if (q.role && t.role !== q.role) return false;
    if (q.accountId && t.accountId !== q.accountId) return false;
    if (q.starredOnly && !t.starred) return false;
    if (q.unreadOnly && !t.messages.some((m) => !m.isRead)) return false;
    if (q.withAttachments && t.attachments.length === 0) return false;
    const pending = t.reminder && !t.reminder.fired;
    if (q.remindersOnly && !pending) return false;
    if (!q.remindersOnly && !words.length && pending && t.reminder?.kind === "snooze") return false;
    if (words.length) {
      if (t.role === "trash" || t.role === "junk") return false; // la recherche ignore corbeille et spam par défaut
      const hay = norm(t.messages.map((m) => `${m.subject} ${m.fromName} ${m.fromAddr} ${m.bodyText ?? htmlToText(m.bodyHtml ?? "")}`).join(" "));
      if (!words.every((w) => hay.includes(w))) return false;
    }
    return true;
  })
    .map(summarize)
    .sort((a, b) => b.lastMessageAt.localeCompare(a.lastMessageAt))
    .slice(q.offset ?? 0, (q.offset ?? 0) + (q.limit ?? 100));
}

const find = (id: number) => {
  const t = T.find((x) => x.id === id);
  if (!t) throw new Error(i18next.t("demo:threadNotFound", { id }));
  return t;
};

export const demoBackend: Backend = {
  demo: true,
  listAccounts: () => delay(structuredClone(ACCOUNTS), 60),
  listFolders: () => delay(folders(), 60),
  listThreads: (q) => delay(listThreads(q)),
  getThread: (id) => {
    const t = find(id);
    const detail: ThreadDetail = { id: t.id, accountId: t.accountId, subject: summarize(t).subject, isStarred: t.starred, messages: structuredClone(t.messages), attachments: structuredClone(t.attachments) };
    return delay(detail, 90);
  },
  setRead: (id, read) => {
    find(id).messages.forEach((m) => (m.isRead = read));
    return delay(undefined, 80);
  },
  setReminder: (id, kind, due) => {
    find(id).reminder = { kind, due };
    return delay(undefined, 80);
  },
  clearReminder: (id) => {
    delete find(id).reminder;
    return delay(undefined, 80);
  },
  setStarred: (id, starred) => {
    find(id).starred = starred;
    return delay(undefined, 80);
  },
  moveThread: (id, _folderId, to) => {
    find(id).role = to;
    return delay(undefined, 200);
  },
  deleteThread: (id) => {
    T.splice(T.indexOf(find(id)), 1);
    return delay(undefined, 200);
  },
  emptyFolder: (id) => {
    for (let i = T.length - 1; i >= 0; i--) if (id === folderId(T[i].accountId, T[i].role)) T.splice(i, 1);
    return delay(undefined, 300);
  },
  confirm: (message) => Promise.resolve(window.confirm(message)),
  syncNow: () => delay(undefined, 600),
  addAccount: () => Promise.reject(new Error(t("demo:unavailable"))),
  addOAuthAccount: () => Promise.reject(new Error(t("demo:unavailable"))),
  reconnectOAuthAccount: () => Promise.reject(new Error(t("demo:unavailable"))),
  detectServers: (email) => {
    const domain = email.split("@")[1]?.toLowerCase() ?? "";
    const tls = (host: string, port: number) => ({ host, port, security: "tls" as const });
    const oauth = domain === "gmail.com" ? ("google" as const) : domain === "outlook.com" ? ("microsoft" as const) : null;
    return delay<Detected>(
      oauth
        ? { provider: oauth, imap: tls(`imap.${domain}`, 993), smtp: tls(`smtp.${domain}`, 465), username: email, oauth, source: "builtin" }
        : { provider: "imap", imap: tls(`imap.${domain}`, 993), smtp: tls(`smtp.${domain}`, 465), username: email, oauth: null, source: "guess" },
      150,
    );
  },
  sendEmail: () => delay<SendReport>({ sentCopy: "server" }, 400),
  saveDraft: () => delay(undefined, 150),
  deleteDraft: () => delay(undefined, 100),
  openDraft: () => Promise.reject(new Error(t("demo:unavailable"))),
  suggestContacts: (_account, query) => {
    const q = query.toLowerCase();
    const seen = new Map<string, { name: string | null; address: string }>();
    for (const t of T) for (const m of t.messages) if (!seen.has(m.fromAddr)) seen.set(m.fromAddr, { name: m.fromName, address: m.fromAddr });
    return delay([...seen.values()].filter((a) => a.address.includes(q) || a.name?.toLowerCase().includes(q)).slice(0, 8), 60);
  },
  oauthStatus: () => delay({ google: false, microsoft: false }, 10),
  setOAuthClient: () => Promise.reject(new Error(t("demo:unavailable"))),
  clearOAuthClient: () => Promise.resolve(),
  archiveLocal: () => Promise.reject(new Error(t("demo:unavailable"))),
  importPick: () => Promise.resolve(null),
  importRun: () => Promise.reject(new Error(t("demo:unavailable"))),
  exportPick: () => Promise.resolve(null),
  exportFolder: () => Promise.reject(new Error(t("demo:unavailable"))),
  createLocalFolder: () => Promise.reject(new Error(t("demo:unavailable"))),
  renameLocalFolder: () => Promise.reject(new Error(t("demo:unavailable"))),
  deleteLocalFolder: () => Promise.reject(new Error(t("demo:unavailable"))),
  unsubscribe: () => delay(undefined, 300),
  setBadge: () => Promise.resolve(),
  setAccountSignature: (id, signature) => {
    const a = ACCOUNTS.find((x) => x.id === id);
    if (a) a.signature = signature;
    return delay(undefined, 60);
  },
  scheduleSend: (accountId, message, sendAt) => {
    const id = Math.max(0, ...QUEUE.map((q) => q.id)) + 1;
    QUEUE.push({ id, accountId, message, sendAt });
    return delay(id, 60);
  },
  listScheduled: () => delay(QUEUE.map((q) => ({ id: q.id, accountId: q.accountId, sendAt: q.sendAt, subject: q.message.subject, to: q.message.to, status: "pending" as const, error: null })), 40),
  cancelScheduled: (id) => {
    const i = QUEUE.findIndex((q) => q.id === id);
    if (i < 0) return Promise.reject(new Error(t("demo:sendNotFound")));
    return delay(QUEUE.splice(i, 1)[0].message, 40);
  },
  sendScheduledNow: (id) => {
    const i = QUEUE.findIndex((q) => q.id === id);
    if (i >= 0) QUEUE.splice(i, 1);
    return delay(undefined, 200);
  },
  addIdentity: (accountId, name, email) => {
    const a = ACCOUNTS.find((x) => x.id === accountId);
    if (a) a.identities.push({ id: Math.max(0, ...ACCOUNTS.flatMap((x) => x.identities.map((i) => i.id))) + 1, name, email });
    return delay(undefined, 60);
  },
  removeIdentity: (id) => {
    for (const a of ACCOUNTS) a.identities = a.identities.filter((i) => i.id !== id);
    return delay(undefined, 40);
  },
  listRules: () => delay(structuredClone(RULES), 40),
  saveRule: (r) => {
    if (!r.name.trim() || !r.query.trim()) return Promise.reject(new Error(t("demo:ruleRequired")));
    const id = r.id || Math.max(0, ...RULES.map((x) => x.id)) + 1;
    const i = RULES.findIndex((x) => x.id === id);
    if (i >= 0) RULES[i] = { ...r, id }; else RULES.push({ ...r, id });
    return delay(id, 60);
  },
  deleteRule: (id) => {
    const i = RULES.findIndex((x) => x.id === id);
    if (i >= 0) RULES.splice(i, 1);
    return delay(undefined, 40);
  },
  runRulesNow: () => delay(0, 200),
  listTemplates: () => delay([...TEMPLATES].sort((a, b) => a.name.localeCompare(b.name)), 40),
  saveTemplate: (t) => {
    const i = TEMPLATES.findIndex((x) => x.name.toLowerCase() === t.name.trim().toLowerCase());
    const saved = { ...t, name: t.name.trim(), id: i >= 0 ? TEMPLATES[i].id : Math.max(0, ...TEMPLATES.map((x) => x.id)) + 1 };
    if (i >= 0) TEMPLATES[i] = saved; else TEMPLATES.push(saved);
    return delay(saved, 60);
  },
  deleteTemplate: (id) => {
    const i = TEMPLATES.findIndex((x) => x.id === id);
    if (i >= 0) TEMPLATES.splice(i, 1);
    return delay(undefined, 40);
  },
  removeAccount: () => Promise.reject(new Error(t("demo:unavailable"))),
  openAttachment: () => delay(undefined, 60),
  inlineImages: () => delay({}, 10),
  lockStatus: () => delay({ ...LOCK }, 20),
  unlockPassword: (password) => (password === "demo1234" ? ((LOCK.locked = false), delay(undefined, 200)) : Promise.reject(new Error(t("demo:wrongPassword")))),
  unlockSystem: () => ((LOCK.locked = false), delay(undefined, 300)),
  resetApp: () => ((LOCK.method = "off"), (LOCK.locked = false), delay(undefined, 200)),
  lockNow: () => ((LOCK.locked = true), delay(undefined, 20)),
  setLock: (method, timeoutSecs) => {
    Object.assign(LOCK, { method, timeoutSecs });
    return delay({ ...LOCK }, 100);
  },
  diskEncryption: () => delay<DiskEncryption>("on", 30),
  mcpInfo: () => delay<McpInfo>({ binaryPath: "/Applications/Envelope.app/Contents/MacOS/envelope-mcp", dbPath: "~/Library/Application Support/dev.envelope.app/envelope.db" }, 40),
};
