// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoBackend } from "../mock";
import { selectionIds, useMail, visibleThreads } from "../store";

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

  it("archiver masque aussitôt, part tout de suite au serveur et propose d'annuler", async () => {
    await boot();
    const spy = vi.spyOn(demoBackend, "moveThread");
    const [first, second] = visibleThreads(useMail.getState());
    useMail.getState().moveTo(first.id, "archive");

    let s = useMail.getState();
    expect(visibleThreads(s).some((t) => t.id === first.id)).toBe(false);
    expect(s.selectedId).toBe(second.id);
    expect(spy).toHaveBeenCalledWith(first.id, expect.any(Number), "archive"); // sans délai d'attente

    await tick(500); // latence de la démo
    s = useMail.getState();
    expect(s.threads.some((t) => t.id === first.id)).toBe(false);
    expect(s.toasts[0]?.message).toBe("Conversation archivée");
  });

  it("la corbeille et le spam suivent le même chemin avec leur destination", async () => {
    await boot();
    const spy = vi.spyOn(demoBackend, "moveThread");
    const [a, b] = visibleThreads(useMail.getState());
    useMail.getState().moveTo(a.id, "trash");
    useMail.getState().moveTo(b.id, "junk");
    await tick(500);
    expect(spy).toHaveBeenCalledWith(a.id, expect.any(Number), "trash");
    expect(spy).toHaveBeenCalledWith(b.id, expect.any(Number), "junk");
  });

  it("depuis les archives, on peut remettre la conversation dans la boîte de réception", async () => {
    await boot();
    const spy = vi.spyOn(demoBackend, "moveThread");
    useMail.getState().setView({ kind: "role", role: "archive" });
    await tick(500);
    const target = visibleThreads(useMail.getState())[0];
    const archiveIds = useMail.getState().folders.filter((f) => f.role === "archive").map((f) => f.id);
    useMail.getState().moveTo(target.id, "archive"); // déjà archivée : rien ne part
    expect(useMail.getState().toasts[0].message).toContain("déjà archivée");
    useMail.getState().moveTo(target.id, "inbox");
    await tick(500);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(archiveIds).toContain(spy.mock.calls[0][1]);
    expect(spy.mock.calls[0][2]).toBe("inbox");
  });

  it("la suppression définitive n'existe que dans la corbeille et le spam, et demande confirmation", async () => {
    await boot();
    const spy = vi.spyOn(demoBackend, "deleteThread");
    const inInbox = visibleThreads(useMail.getState())[0];
    await useMail.getState().deleteForever(inInbox.id); // vue Réception : refusé
    expect(spy).not.toHaveBeenCalled();

    useMail.getState().setView({ kind: "role", role: "trash" });
    await tick(500);
    const target = visibleThreads(useMail.getState())[0];
    vi.spyOn(demoBackend, "confirm").mockResolvedValueOnce(false);
    await useMail.getState().deleteForever(target.id);
    expect(spy).not.toHaveBeenCalled(); // refusé à la confirmation

    vi.spyOn(demoBackend, "confirm").mockResolvedValueOnce(true);
    const done = useMail.getState().deleteForever(target.id);
    await tick(500);
    await done;
    expect(spy).toHaveBeenCalledWith(target.id, expect.any(Number));
  });

  it("vider la corbeille vide tous les dossiers corbeille de la vue", async () => {
    await boot();
    const spy = vi.spyOn(demoBackend, "emptyFolder");
    useMail.getState().setView({ kind: "role", role: "trash" });
    await tick(500);
    vi.spyOn(demoBackend, "confirm").mockResolvedValueOnce(true);
    const done = useMail.getState().emptyView();
    await tick(1000);
    await done;
    const trashFolders = useMail.getState().folders.filter((f) => f.role === "trash").length;
    expect(spy).toHaveBeenCalledTimes(trashFolders);
  });

  it("sélection multiple : plage, ajout, tout, et une action s'applique à tout le lot", async () => {
    await boot();
    const list = visibleThreads(useMail.getState());
    const st = () => useMail.getState();
    st().select(list[0].id);
    st().rangeCheck(list[2].id);
    expect([...st().checked]).toEqual([list[0].id, list[1].id, list[2].id]);
    expect(selectionIds(st())).toHaveLength(3);
    st().toggleCheck(list[1].id); // retire une conversation
    expect(selectionIds(st())).toEqual([list[0].id, list[2].id]);
    st().checkAll();
    expect(st().checked.size).toBe(list.length);

    const spy = vi.spyOn(demoBackend, "moveThread");
    st().select(list[0].id);
    st().rangeCheck(list[1].id);
    st().moveTo(selectionIds(st()), "archive");
    await tick(500);
    expect(spy).toHaveBeenCalledTimes(2);
    expect(st().checked.size).toBe(0);
    expect(st().toasts[0].message).toContain("2 conversations archivées");
  });

  it("une sélection d'une seule conversation redevient la conversation ouverte", async () => {
    await boot();
    const [a, b] = visibleThreads(useMail.getState());
    useMail.getState().select(a.id);
    useMail.getState().toggleCheck(b.id);
    expect(useMail.getState().checked.size).toBe(2);
    useMail.getState().toggleCheck(a.id);
    expect(useMail.getState().checked.size).toBe(0);
    expect(useMail.getState().selectedId).toBe(b.id);
  });

  it("un échec serveur restaure la conversation et prévient l'utilisateur", async () => {
    await boot();
    vi.spyOn(demoBackend, "moveThread").mockRejectedValue(new Error("serveur injoignable"));
    const target = visibleThreads(useMail.getState())[0];
    useMail.getState().moveTo(target.id, "archive");
    await tick(100);
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
