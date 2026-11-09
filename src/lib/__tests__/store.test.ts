// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoBackend } from "../mock";
import { useMail, visibleThreads } from "../store";

/** Avance le temps fictif : les « latences » de la démo et la fenêtre d'annulation. */
const tick = (ms: number) => vi.advanceTimersByTimeAsync(ms);

async function boot() {
  const p = useMail.getState().enterDemo();
  await tick(1000);
  await p;
}

const initial = useMail.getState();
beforeEach(() => {
  useMail.setState(initial, true); // le store est un singleton : on repart d'un état neuf
  vi.useFakeTimers();
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("store (boîte de démonstration)", () => {
  it("charge la boîte unifiée et sélectionne la première conversation", async () => {
    await boot();
    const s = useMail.getState();
    expect(s.threads.length).toBeGreaterThan(10);
    expect(s.selectedId).toBe(visibleThreads(s)[0].id);
    expect(s.accounts).toHaveLength(2);
  });

  it("archiver masque aussitôt, sélectionne la suivante, puis valide après la fenêtre d'annulation", async () => {
    await boot();
    const spy = vi.spyOn(demoBackend, "archive");
    const [first, second] = visibleThreads(useMail.getState());
    useMail.getState().archive(first.id);

    let s = useMail.getState();
    expect(visibleThreads(s).some((t) => t.id === first.id)).toBe(false);
    expect(s.selectedId).toBe(second.id);
    expect(s.toasts[0]?.actionLabel).toBe("Annuler");
    expect(spy).not.toHaveBeenCalled(); // rien n'est parti tant que l'annulation est possible

    await tick(5700); // 5 s de fenêtre + latence de la démo
    expect(spy).toHaveBeenCalledWith(first.id, expect.any(Number));
    s = useMail.getState();
    expect(s.threads.some((t) => t.id === first.id)).toBe(false);
  });

  it("annuler restaure la conversation et n'appelle jamais le serveur", async () => {
    await boot();
    const spy = vi.spyOn(demoBackend, "archive");
    const target = visibleThreads(useMail.getState())[0];
    useMail.getState().archive(target.id);
    useMail.getState().undo();
    await tick(6000);
    expect(spy).not.toHaveBeenCalled();
    expect(visibleThreads(useMail.getState()).some((t) => t.id === target.id)).toBe(true);
  });

  it("un échec serveur restaure la conversation et prévient l'utilisateur", async () => {
    await boot();
    vi.spyOn(demoBackend, "archive").mockRejectedValue(new Error("serveur injoignable"));
    const target = visibleThreads(useMail.getState())[0];
    useMail.getState().archive(target.id);
    await tick(5200);
    const s = useMail.getState();
    expect(visibleThreads(s).some((t) => t.id === target.id)).toBe(true);
    expect(s.toasts[0]).toMatchObject({ tone: "error" });
    expect(s.toasts[0].message).toContain("serveur injoignable");
  });

  it("suivre / lire sont optimistes et reviennent en arrière en cas d'échec", async () => {
    await boot();
    const t = useMail.getState().threads.find((x) => !x.isStarred)!;
    useMail.getState().toggleStar(t.id);
    expect(useMail.getState().threads.find((x) => x.id === t.id)!.isStarred).toBe(true);

    vi.spyOn(demoBackend, "setStarred").mockRejectedValue(new Error("non"));
    useMail.getState().toggleStar(t.id); // retire l'étoile… puis échoue
    expect(useMail.getState().threads.find((x) => x.id === t.id)!.isStarred).toBe(false);
    await tick(10);
    expect(useMail.getState().threads.find((x) => x.id === t.id)!.isStarred).toBe(true); // revenu à l'état serveur
  });

  it("la recherche ignore les accents et les réponses tardives d'une ancienne requête", async () => {
    await boot();
    useMail.getState().setQuery("zzz");
    useMail.getState().setQuery("recu"); // sans accent : trouve « reçu »
    await tick(900);
    const s = useMail.getState();
    expect(s.query).toBe("recu");
    const subjects = s.threads.map((t) => t.subject);
    expect(subjects).toContain("Reçu de paiement — 49,00 €");
    expect(subjects).toContain("Question sur la facture #2026-0142"); // « reçu » dans le corps du message
    expect(subjects).toHaveLength(2);
  });

  it("Smart Inbox : l'onglet filtre la liste", async () => {
    await boot();
    useMail.getState().setCategory("newsletters");
    const list = visibleThreads(useMail.getState());
    expect(list.length).toBeGreaterThan(0);
    expect(list.every((t) => /lalettretech|rust-france/.test(t.fromAddr ?? ""))).toBe(true);
  });
});
