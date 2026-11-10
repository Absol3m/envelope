// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import type { ThreadDetail } from "../api";
import { attachmentError, draftFrom, emailHtml, formatRecipient, htmlToPlain, parseRecipients, textToHtml, toOutgoing, withPrefix } from "../compose";

const msg = (over: Partial<ThreadDetail["messages"][number]>) => ({
  id: 1, messageId: "m1@x", references: [] as string[], folderId: 1, subject: "Budget", fromName: "Alice", fromAddr: "alice@x.fr",
  to: [{ name: null, address: "me@x.fr" }, { name: "Bob", address: "bob@x.fr" }], cc: [{ name: null, address: "carol@x.fr" }],
  date: "2026-03-02T10:00:00Z", bodyText: "Bonjour\n\nÀ demain", bodyHtml: null, isRead: true, isStarred: false, ...over,
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
