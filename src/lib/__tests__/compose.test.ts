// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import type { ThreadDetail } from "../api";
import type { Draft } from "../compose";
import { applyTemplate, knownAlias, senderOptions, templateBody, attachmentError, draftFrom, forgotAttachment, emailHtml, emptyDraft, signatureHtml, swapSignature, formatRecipient, htmlToPlain, isNeutralColor, parseRecipients, textToHtml, toOutgoing, withPrefix } from "../compose";

const msg = (over: Partial<ThreadDetail["messages"][number]>) => ({
  id: 1, messageId: "m1@x", references: [] as string[], folderId: 1, subject: "Budget", fromName: "Alice", fromAddr: "alice@x.fr",
  to: [{ name: null, address: "me@x.fr" }, { name: "Bob", address: "bob@x.fr" }], cc: [{ name: null, address: "carol@x.fr" }],
  date: "2026-03-02T10:00:00Z", bodyText: "Bonjour\n\nÀ demain", bodyHtml: null, isRead: true, isStarred: false, unsubscribe: null, authFail: null, ...over,
});
const thread = (messages = [msg({})]): ThreadDetail => ({ id: 1, accountId: "a", subject: "Re: Budget", isStarred: false, messages, attachments: [] });

describe("parseRecipients", () => {
  it("accepte adresses nues, « Nom <adresse> » et séparateurs mixtes", () => {
    const { ok, rejected } = parseRecipients('a@x.fr, "Dupont, Jean" <j@x.fr>; Bob <b@x.fr>\nnope');
    expect(ok.map((r) => r.address)).toEqual(["a@x.fr", "j@x.fr", "b@x.fr"]);
    expect(ok[1].name).toBe("Dupont, Jean");
    expect(rejected).toEqual(["nope"]);
  });
  it("n'envoie pas de nom contenant des caractères spéciaux", () => {
    expect(formatRecipient({ address: "j@x.fr", name: "Dupont, Jean" })).toBe("j@x.fr");
    expect(formatRecipient({ address: "b@x.fr", name: "Bob Martin" })).toBe("Bob Martin <b@x.fr>");
  });
});

describe("corps HTML", () => {
  it("texte → paragraphes, avec échappement", () => {
    expect(textToHtml("a < b\nsuite\n\nautre")).toBe("<p>a &lt; b<br>suite</p><p>autre</p>");
  });
  it("HTML → texte brut : listes, citations, liens", () => {
    const html = '<p>Salut <strong>toi</strong></p><ul><li>un</li><li>deux</li></ul><ol><li>a</li></ol><p><a href="https://x.fr">site</a> <a href="mailto:a@x.fr">a@x.fr</a></p><blockquote><p>cité</p></blockquote>';
    expect(htmlToPlain(html)).toBe("Salut toi\n\n- un\n- deux\n\n1. a\n\nsite (https://x.fr) a@x.fr\n\n> cité");
  });
  it("un message vide reste vide", () => {
    expect(htmlToPlain("")).toBe("");
    expect(htmlToPlain("<p></p>")).toBe("");
  });
  it("styles en ligne pour les clients mail", () => {
    const out = emailHtml("<p>x</p><blockquote><p>q</p></blockquote>");
    expect(out).toContain('<p style="margin: 0px 0px 12px;">x</p>');
    expect(out).toContain("border-left");
    expect(out.startsWith("<div style=")).toBe(true);
  });
  it("les <span> sans attribut sont retirés, ceux qui portent une couleur restent", () => {
    const out = emailHtml('<p><span>a</span> <span style="color: rgb(220, 38, 38);">b</span></p>');
    expect(out).toContain('a <span style="color: rgb(220, 38, 38);">b</span>');
    expect(out).not.toContain("<span>");
  });
  it("couleurs neutres : noir, blanc, gris (pas le gris bleuté de la palette)", () => {
    for (const c of ["rgb(0, 0, 0)", "rgb(34, 34, 34)", "rgba(255, 255, 255, 0.9)", "Black", "windowtext"]) expect(isNeutralColor(c)).toBe(true);
    for (const c of ["rgb(211, 47, 47)", "rgb(107, 107, 120)", "navy", ""]) expect(isNeutralColor(c)).toBe(false);
  });
  it("les citations de mails HTML perdent images et scripts", () => {
    const d = draftFrom("reply", thread([msg({ bodyText: null, bodyHtml: '<style>p{}</style><p>ok<img src="https://track/x.gif"><script>1</script></p>' })]), "me@x.fr");
    expect(d.html).not.toMatch(/img|script|style|track/);
    expect(d.html).toContain("ok");
  });
});

describe("pièces jointes", () => {
  const a = (size: number) => ({ id: String(size), name: "f", mimeType: "x/y", size, dataBase64: "" });
  it("accepte dans la limite, refuse au-delà de 25 Mo cumulés ou de 20 fichiers", () => {
    expect(attachmentError([a(1)], [{ name: "b", size: 1000 }])).toBeNull();
    expect(attachmentError([a(20 * 1024 * 1024)], [{ name: "b", size: 6 * 1024 * 1024 }])).toMatch(/25 Mo/);
    expect(attachmentError(Array.from({ length: 20 }, () => a(1)), [{ name: "b", size: 1 }])).toMatch(/Trop/);
  });
  it("part avec le message, au format attendu par le serveur", () => {
    const d = { ...draftFrom("forward", thread(), "me@x.fr"), attachments: [{ id: "1", name: "devis.pdf", mimeType: "application/pdf", size: 3, dataBase64: "AAEC" }] };
    expect(toOutgoing(d).attachments).toEqual([{ filename: "devis.pdf", mimeType: "application/pdf", dataBase64: "AAEC" }]);
  });
});

describe("withPrefix", () => {
  it("ne cumule pas Re:/TR:/Fwd:", () => {
    expect(withPrefix("Re", "RE : Re: Budget")).toBe("Re: Budget");
    expect(withPrefix("Fwd", "TR: Budget")).toBe("Fwd: Budget");
  });
});

describe("draftFrom", () => {
  it("répond à l'expéditeur avec citation et en-têtes de threading", () => {
    const d = draftFrom("reply", thread([msg({ references: ["m0@x"] })]), "me@x.fr");
    expect(d.to).toEqual([{ address: "alice@x.fr", name: "Alice" }]);
    expect(d.cc).toEqual([]);
    expect(d.subject).toBe("Re: Budget");
    expect(d.html).toContain("Alice &lt;alice@x.fr&gt; a écrit :");
    expect(d.html).toContain("<blockquote><p>Bonjour</p><p>À demain</p></blockquote>");
    expect(toOutgoing(d).bodyText).toContain("a écrit :\n\n> Bonjour\n>\n> À demain");
    expect(d.inReplyTo).toBe("m1@x");
    expect(d.references).toEqual(["m0@x", "m1@x"]);
  });
  it("répondre à tous : ajoute les autres destinataires sans soi-même ni doublon", () => {
    const d = draftFrom("replyAll", thread(), "ME@x.fr");
    expect(d.to.map((r) => r.address)).toEqual(["alice@x.fr"]);
    expect(d.cc.map((r) => r.address)).toEqual(["bob@x.fr", "carol@x.fr"]);
  });
  it("si le dernier message est de nous, on écrit à ses destinataires", () => {
    const d = draftFrom("reply", thread([msg({ fromAddr: "me@x.fr", fromName: null, to: [{ name: null, address: "alice@x.fr" }] })]), "me@x.fr");
    expect(d.to.map((r) => r.address)).toEqual(["alice@x.fr"]);
  });
  it("transfert : objet préfixé, corps cité, aucun destinataire ni In-Reply-To", () => {
    const d = draftFrom("forward", thread(), "me@x.fr");
    expect(d.subject).toBe("Fwd: Budget");
    expect(d.html).toContain("Message transféré");
    expect(d.to).toEqual([]);
    expect(d.inReplyTo).toBeUndefined();
  });
  it("retombe sur le texte du HTML quand il n'y a pas de version texte", () => {
    const d = draftFrom("reply", thread([msg({ bodyText: null, bodyHtml: "<p>Salut</p><p>Toi</p>" })]), "me@x.fr");
    expect(d.html).toContain("<blockquote><p>Salut</p><p>Toi</p></blockquote>");
  });
});

describe("signature", () => {
  it("bloc HTML avec séparateur, échappé, vide si pas de signature", () => {
    expect(signatureHtml("Alex\n<Atelier>")).toBe("<p>--<br>Alex<br>&lt;Atelier&gt;</p>");
    expect(signatureHtml("  \n ")).toBe("");
  });
  it("s'insère dans un nouveau message et sous le curseur d'une réponse, avant la citation", () => {
    expect(emptyDraft("a", "Alex").html).toBe("<p></p><p>--<br>Alex</p>");
    expect(emptyDraft("a").html).toBe("");
    const d = draftFrom("reply", thread([msg({ bodyText: "salut", bodyHtml: null })]), "me@x.fr", "Alex");
    expect(d.html.indexOf("Alex")).toBeGreaterThan(-1);
    expect(d.html.indexOf("Alex")).toBeLessThan(d.html.indexOf("<blockquote>"));
  });
  it("change avec le compte expéditeur, sans écraser une signature retouchée", () => {
    const html = `<p>Bonjour</p>${signatureHtml("A")}`;
    expect(swapSignature(html, "A", "B")).toBe(`<p>Bonjour</p>${signatureHtml("B")}`);
    expect(swapSignature("<p>Bonjour</p><p>--<br>A modifiée</p>", "A", "B")).toBe("<p>Bonjour</p><p>--<br>A modifiée</p>");
    expect(swapSignature("", "", "B")).toBe(`<p></p>${signatureHtml("B")}`);
  });
});

describe("rappel de pièce jointe oubliée", () => {
  const draft = (html: string, over: Partial<Draft> = {}) => ({ ...emptyDraft("a"), html, ...over });
  it("repère une mention dans le texte ou l'objet, en français comme en anglais", () => {
    expect(forgotAttachment(draft("<p>Veuillez trouver le devis ci-joint.</p>"))).toBe(true);
    expect(forgotAttachment(draft("<p>Voir la pièce jointe</p>"))).toBe(true);
    expect(forgotAttachment(draft("<p>Je vous joins le contrat</p>"))).toBe(true);
    expect(forgotAttachment(draft("<p>Please find the attached file</p>"))).toBe(true);
    expect(forgotAttachment(draft("<p>Bonjour</p>", { subject: "Devis en pièce jointe" }))).toBe(true);
  });
  it("ne se déclenche pas sans mention, ni si une pièce est déjà jointe", () => {
    expect(forgotAttachment(draft("<p>Bonjour, à demain.</p>"))).toBe(false);
    expect(forgotAttachment(draft("<p>Le joint de culasse est cassé</p>"))).toBe(false); // « joint » nom commun
    expect(forgotAttachment(draft("<p>ci-joint</p>", { attachments: [{ id: "1", name: "a.pdf", mimeType: "application/pdf", size: 1, dataBase64: "" }] }))).toBe(false);
  });
  it("ignore la citation et la signature", () => {
    expect(forgotAttachment(draft("<p>Merci !</p><p>--<br>Envoyé avec pièce jointe</p>"))).toBe(false);
    expect(forgotAttachment(draft("<p>Merci !</p><blockquote><p>voir pièce jointe</p></blockquote>"))).toBe(false);
  });
});

describe("modèles", () => {
  const tpl = { subject: "Relance", bodyHtml: "<p>Bonjour,</p>" };
  it("place le corps avant la signature et donne l'objet s'il est vide", () => {
    const d = applyTemplate(emptyDraft("a", "Alice"), tpl);
    expect(d.subject).toBe("Relance");
    expect(d.html.startsWith("<p>Bonjour,</p><p>--")).toBe(true);
  });
  it("garde l'objet déjà saisi et le texte déjà écrit", () => {
    const d = applyTemplate({ ...emptyDraft("a"), subject: "Mon objet", html: "<p>Début</p>" }, tpl);
    expect(d.subject).toBe("Mon objet");
    expect(d.html).toBe("<p>Bonjour,</p><p>Début</p>");
  });
});

describe("corps de modèle", () => {
  it("retire la signature du compte et la citation d'une réponse", () => {
    const d = { ...emptyDraft("a", "Alice"), html: `<p>Merci !</p>${signatureHtml("Alice")}<p>Le 1er, Bob a écrit :</p><blockquote><p>texte cité</p></blockquote>` };
    expect(templateBody(d, "Alice")).toBe("<p>Merci !</p>");
  });
  it("garde un corps sans signature ni citation, et ignore le paragraphe vide initial", () => {
    expect(templateBody({ ...emptyDraft("a", "Alice"), html: `<p></p><p>Bonjour</p>${signatureHtml("Alice")}` }, "Alice")).toBe("<p>Bonjour</p>");
  });
});

describe("réponse depuis un alias", () => {
  it("répond depuis l'alias auquel le message a été envoyé, sans l'ajouter en copie", () => {
    const t = thread([msg({ to: [{ name: null, address: "contact@x.fr" }, { name: "Bob", address: "bob@x.fr" }] })]);
    const d = draftFrom("replyAll", t, "me@x.fr", "", ["contact@x.fr"]);
    expect(d.fromEmail).toBe("contact@x.fr");
    expect(d.cc.map((r) => r.address)).toEqual(["bob@x.fr", "carol@x.fr"]);
  });
  it("garde l'adresse du compte quand le message n'a pas été adressé à un alias", () => {
    expect(draftFrom("reply", thread(), "me@x.fr", "", ["contact@x.fr"]).fromEmail).toBeUndefined();
  });
  it("un message envoyé depuis un alias est « de moi » : on répond à ses destinataires", () => {
    const t = thread([msg({ fromAddr: "contact@x.fr", to: [{ name: null, address: "alice@x.fr" }] })]);
    expect(draftFrom("reply", t, "me@x.fr", "", ["contact@x.fr"]).to.map((r) => r.address)).toEqual(["alice@x.fr"]);
  });
  it("l'alias part dans le message à envoyer", () => {
    expect(toOutgoing({ ...emptyDraft("a"), fromEmail: "contact@x.fr" }).fromEmail).toBe("contact@x.fr");
  });
});

describe("expéditeurs", () => {
  const acct = (identities: { id: number; name: string; email: string }[]) => ({ id: "a", email: "me@x.fr", displayName: "Moi", provider: "imap" as const, authKind: "password" as const, syncStatus: "idle" as const, syncError: null, lastSyncAt: null, signature: "", copySent: true, pendingCopies: 0, pendingCopyError: null, identities });
  it("liste chaque compte puis ses alias", () => {
    const o = senderOptions([acct([{ id: 1, name: "Pro", email: "contact@x.fr" }])]);
    expect(o.map((x) => [x.alias, x.label])).toEqual([["", "Moi <me@x.fr>"], ["contact@x.fr", "Pro <contact@x.fr>"]]);
  });
  it("un alias supprimé n'est plus reconnu", () => {
    const a = acct([{ id: 1, name: "", email: "Contact@x.fr" }]);
    expect(knownAlias(a, "contact@x.fr")).toBe("Contact@x.fr");
    expect(knownAlias(acct([]), "contact@x.fr")).toBeUndefined();
    expect(knownAlias(a, undefined)).toBeUndefined();
  });
});
