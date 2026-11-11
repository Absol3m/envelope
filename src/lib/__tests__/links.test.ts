// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend } from "../api";
import { linkRisk, openLink } from "../links";
import { demoBackend } from "../mock";
import { useMail } from "../store";

describe("liens suspects", () => {
  it("texte annonçant un autre site que la destination", () => {
    expect(linkRisk("https://evil.test/login", "www.paypal.com")).toContain("« www.paypal.com », mais il mène à « evil.test »");
    expect(linkRisk("https://evil.test/", "https://banque.fr/connexion")).toContain("« banque.fr »");
    expect(linkRisk("https://evil.test/", "banque.fr")).not.toBeNull();
  });
  it("même site ou sous-domaine : rien à signaler", () => {
    expect(linkRisk("https://www.paypal.com/signin", "paypal.com")).toBeNull();
    expect(linkRisk("https://paypal.com/signin", "www.paypal.com")).toBeNull();
    expect(linkRisk("https://click.paypal.com/x?id=1", "https://paypal.com")).toBeNull();
  });
  it("un texte qui n'est pas une adresse n'est pas comparé (boutons, newsletters, fichiers)", () => {
    expect(linkRisk("https://tracker.mailer.test/c/1", "Cliquez ici")).toBeNull();
    expect(linkRisk("https://tracker.mailer.test/c/1", "Voir la facture n° 12.5")).toBeNull();
    expect(linkRisk("https://x.test/f", "rapport.pdf")).toBeNull();
    expect(linkRisk("https://x.test/f", "")).toBeNull();
  });
  it("identifiant avant le site, adresse numérique, lettres imitées", () => {
    expect(linkRisk("https://paypal.com@evil.test/", "ok")).toContain("identifiant");
    expect(linkRisk("http://192.168.1.10/login", "ok")).toContain("adresse numérique");
    expect(linkRisk("https://раураl.com/", "ok")).toContain("non latins"); // cyrillique
  });
  it("schémas non web signalés, mailto accepté", () => {
    for (const bad of ["javascript:alert(1)", "file:///etc/passwd", "pas une url"]) expect(linkRisk(bad, "x")).toContain("pas une adresse web");
    expect(linkRisk("mailto:a@x.fr", "a@x.fr")).toBeNull();
  });
});

describe("ouverture d'un lien", () => {
  const open = vi.spyOn(window, "open").mockImplementation(() => null);
  let confirm: ReturnType<typeof vi.fn>;
  beforeEach(() => {
    open.mockClear();
    confirm = vi.fn(() => Promise.resolve(false));
    useMail.setState({ backend: { ...demoBackend, confirm } as Backend });
  });
  it("lien sûr : ouvert sans question", async () => {
    await openLink("https://paypal.com/signin", "Se connecter");
    expect(confirm).not.toHaveBeenCalled();
    expect(open).toHaveBeenCalledOnce();
  });
  it("lien douteux : confirmation, et rien ne s'ouvre si l'on refuse", async () => {
    await openLink("https://evil.test/", "www.paypal.com");
    expect(confirm).toHaveBeenCalledOnce();
    expect(confirm.mock.calls[0][0]).toContain("evil.test");
    expect(open).not.toHaveBeenCalled();
  });
  it("lien douteux confirmé : ouvert", async () => {
    confirm.mockResolvedValueOnce(true);
    await openLink("http://192.168.0.1/", "http://192.168.0.1/"); // texte brut : même adresse partout
    expect(open).toHaveBeenCalledOnce();
  });
});
