// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend } from "../api";
import { linkRisk, openLink } from "../links";
import { demoBackend } from "../mock";
import { useMail } from "../store";

describe("liens suspects", () => {
  afterEach(() => useMail.setState({ backend: demoBackend }));
  it("texte annonçant un autre site que la destination", async () => {
    expect(await linkRisk("https://evil.test/login", "www.paypal.com")).toContain("« www.paypal.com », mais il mène à « evil.test »");
    expect(await linkRisk("https://evil.test/", "https://banque.fr/connexion")).toContain("« banque.fr »");
    expect(await linkRisk("https://evil.test/", "banque.fr")).not.toBeNull();
  });
  it("même site ou sous-domaine : rien à signaler", async () => {
    expect(await linkRisk("https://www.paypal.com/signin", "paypal.com")).toBeNull();
    expect(await linkRisk("https://paypal.com/signin", "www.paypal.com")).toBeNull();
    expect(await linkRisk("https://click.paypal.com/x?id=1", "https://paypal.com")).toBeNull();
  });
  it("l'avis vient du domaine enregistrable du moteur, pas d'un simple sous-domaine (github.io : sites distincts)", async () => {
    const registrableDomain = (h: string) => Promise.resolve(({ "github.io": "github.io", "monsite.github.io": "monsite.github.io", "autre.github.io": "autre.github.io", "a.monsite.github.io": "monsite.github.io" } as Record<string, string>)[h] ?? h);
    useMail.setState({ backend: { ...demoBackend, registrableDomain } });
    expect(await linkRisk("https://monsite.github.io/", "github.io")).not.toBeNull();
    expect(await linkRisk("https://autre.github.io/", "https://monsite.github.io")).not.toBeNull();
    expect(await linkRisk("https://a.monsite.github.io/", "https://monsite.github.io")).toBeNull();
  });
  it("moteur injoignable : lien douteux (confirmation), pas d'exception", async () => {
    useMail.setState({ backend: { ...demoBackend, registrableDomain: () => Promise.reject(new Error("ipc")) } });
    expect(await linkRisk("https://evil.test/", "banque.fr")).not.toBeNull();
  });
  it("un texte qui n'est pas une adresse n'est pas comparé (boutons, newsletters, fichiers)", async () => {
    expect(await linkRisk("https://tracker.mailer.test/c/1", "Cliquez ici")).toBeNull();
    expect(await linkRisk("https://tracker.mailer.test/c/1", "Voir la facture n° 12.5")).toBeNull();
    expect(await linkRisk("https://x.test/f", "rapport.pdf")).toBeNull();
    expect(await linkRisk("https://x.test/f", "")).toBeNull();
  });
  it("identifiant avant le site, adresse numérique, lettres imitées", async () => {
    expect(await linkRisk("https://paypal.com@evil.test/", "ok")).toContain("identifiant");
    expect(await linkRisk("http://192.168.1.10/login", "ok")).toContain("adresse numérique");
    expect(await linkRisk("https://раураl.com/", "ok")).toContain("non latins"); // cyrillique
  });
  it("schémas non web signalés, mailto accepté", async () => {
    for (const bad of ["javascript:alert(1)", "file:///etc/passwd", "pas une url"]) expect(await linkRisk(bad, "x")).toContain("pas une adresse web");
    expect(await linkRisk("mailto:a@x.fr", "a@x.fr")).toBeNull();
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
