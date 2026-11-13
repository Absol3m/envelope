// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, DraftData } from "../api";
import { emptyDraft } from "../compose";
import { SAVE_DELAY_MS, setServerDrafts, useComposer } from "../composer";
import { demoBackend } from "../mock";
import { useMail } from "../store";

const tick = (ms: number) => vi.advanceTimersByTimeAsync(ms);
let n = 0;
let saveDraft: ReturnType<typeof vi.fn>;
let deleteDraft: ReturnType<typeof vi.fn>;
let release: (() => void) | null;

/** Serveur factice : `saveDraft` peut être retenu (`hold`) pour simuler un enregistrement lent. */
function fakeBackend(opts: { hold?: boolean; remote?: DraftData } = {}) {
  saveDraft = vi.fn(() => (opts.hold ? new Promise<void>((r) => { release = r; }) : Promise.resolve()));
  deleteDraft = vi.fn(() => Promise.resolve());
  useMail.setState({ backend: { ...demoBackend, demo: false, saveDraft, deleteDraft, openDraft: () => Promise.resolve(opts.remote!) } as Backend });
}
const open = (over: Partial<ReturnType<typeof emptyDraft>> = {}) => {
  const id = `brouillon-${++n}`;
  useComposer.setState({ draft: { ...emptyDraft("a"), id, ...over }, syncState: "idle" });
  return id;
};
const edit = (subject: string) => useComposer.getState().update({ subject });

beforeEach(() => {
  vi.useFakeTimers();
  setServerDrafts(true);
  release = null;
  fakeBackend();
});
afterEach(async () => {
  // Un enregistrement retenu par le test doit se terminer, sinon il bloquerait les suivants (un seul à la fois).
  for (let i = 0; i < 3 && release; i++) {
    const r = release;
    release = null;
    r();
    await tick(10);
  }
  useComposer.setState({ draft: null });
  vi.useRealTimers();
});

describe("brouillons enregistrés sur le serveur", () => {
  it("enregistre quand on s'arrête d'écrire, pas à chaque frappe", async () => {
    const id = open();
    edit("Bon");
    await tick(SAVE_DELAY_MS - 1000);
    edit("Bonjour"); // la frappe relance le délai
    await tick(SAVE_DELAY_MS - 1);
    expect(saveDraft).not.toHaveBeenCalled();
    await tick(2);
    expect(saveDraft).toHaveBeenCalledTimes(1);
    const [account, message] = saveDraft.mock.calls[0];
    expect(account).toBe("a");
    expect(message).toMatchObject({ draftId: id, subject: "Bonjour" });
    expect(useComposer.getState().syncState).toBe("saved");
  });

  it("n'enregistre jamais un brouillon vide, ni quand le réglage est désactivé", async () => {
    open();
    useComposer.getState().update({ html: "<p></p>" });
    await tick(SAVE_DELAY_MS + 100);
    expect(saveDraft).not.toHaveBeenCalled();
    setServerDrafts(false);
    edit("Un objet");
    await tick(SAVE_DELAY_MS + 100);
    expect(saveDraft).not.toHaveBeenCalled();
  });

  it("une modification pendant un enregistrement en cours donne un seul suivant, avec la dernière version", async () => {
    fakeBackend({ hold: true });
    open();
    edit("v1");
    await tick(SAVE_DELAY_MS + 10); // l'enregistrement de v1 démarre et reste en attente
    expect(saveDraft).toHaveBeenCalledTimes(1);
    edit("v2");
    await tick(SAVE_DELAY_MS + 10);
    edit("v3");
    await tick(SAVE_DELAY_MS + 10);
    expect(saveDraft).toHaveBeenCalledTimes(1); // une seule opération à la fois
    release!();
    await tick(10);
    expect(saveDraft).toHaveBeenCalledTimes(2);
    expect(saveDraft.mock.calls[1][1]).toMatchObject({ subject: "v3" });
  });

  it("fermer enregistre aussitôt la version finale ; tout effacer supprime le brouillon du serveur", async () => {
    const id = open();
    edit("À finir");
    useComposer.getState().close();
    await tick(10);
    expect(saveDraft).toHaveBeenCalledTimes(1);
    expect(saveDraft.mock.calls[0][1]).toMatchObject({ draftId: id, subject: "À finir" });
    // Rouvert, vidé, fermé : le brouillon n'a plus de raison d'exister sur le serveur.
    useComposer.setState({ draft: { ...emptyDraft("a"), id } });
    useComposer.getState().close();
    expect(deleteDraft).toHaveBeenCalledWith("a", id);
  });

  it("abandonner supprime du serveur seulement ce qui y est ; un enregistrement en cours est rattrapé", async () => {
    const id = open();
    useComposer.getState().discard();
    expect(deleteDraft).not.toHaveBeenCalled(); // jamais enregistré : rien à supprimer

    fakeBackend({ hold: true });
    const slow = open();
    edit("En cours");
    await tick(SAVE_DELAY_MS + 10);
    useComposer.getState().discard(); // abandonné pendant l'enregistrement
    release!();
    await tick(10);
    expect(deleteDraft).toHaveBeenCalledWith("a", slow);
    expect(id).not.toBe(slow);
  });

  it("changer de compte expéditeur retire le brouillon du serveur de l'ancien compte", async () => {
    const id = open();
    edit("Pour le pro");
    await tick(SAVE_DELAY_MS + 10);
    useComposer.getState().update({ accountId: "b" });
    expect(deleteDraft).toHaveBeenCalledWith("a", id);
    await tick(SAVE_DELAY_MS + 10);
    expect(saveDraft).toHaveBeenLastCalledWith("b", expect.objectContaining({ draftId: id }));
  });

  it("rouvrir un brouillon du serveur ne le renvoie pas tant qu'il n'a pas changé", async () => {
    const remote: DraftData = { id: "distant-1", accountId: "a", fromEmail: "me@x.fr", to: ["Bob <bob@y.fr>"], cc: [], bcc: [], subject: "Devis", bodyHtml: "<p>Bonjour</p>", bodyText: "Bonjour", inReplyTo: null, references: [], attachments: [{ filename: "a.pdf", mimeType: "application/pdf", dataBase64: "AAEC" }] };
    fakeBackend({ remote });
    await useComposer.getState().openDraft(7);
    const d = useComposer.getState().draft!;
    expect(d).toMatchObject({ id: "distant-1", subject: "Devis", mode: "new" });
    expect(d.to).toEqual([{ address: "bob@y.fr", name: "Bob" }]);
    expect(d.attachments[0]).toMatchObject({ name: "a.pdf", size: 3 });
    await tick(SAVE_DELAY_MS + 100);
    expect(saveDraft).not.toHaveBeenCalled();
    edit("Devis révisé");
    await tick(SAVE_DELAY_MS + 100);
    expect(saveDraft).toHaveBeenCalledTimes(1);
  });

  it("l'envoi transmet l'identifiant du brouillon, que le serveur supprime après un envoi réussi", async () => {
    const id = open({ to: [{ address: "bob@y.fr", name: null }], subject: "Salut" });
    const sendEmail = vi.fn(() => Promise.resolve());
    useMail.setState({ backend: { ...useMail.getState().backend, sendEmail } as Backend });
    useComposer.getState().send();
    await tick(5000 + 100);
    expect(sendEmail).toHaveBeenCalledTimes(1);
    expect((sendEmail.mock.calls[0] as unknown[])[1]).toMatchObject({ draftId: id });
  });
});
