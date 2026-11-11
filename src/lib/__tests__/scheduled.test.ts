// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, OutgoingMessage } from "../api";
import { draftFromQueued, emptyDraft, scheduleOptions } from "../compose";
import { useComposer } from "../composer";
import { demoBackend } from "../mock";
import { useMail } from "../store";

let scheduleSend: ReturnType<typeof vi.fn>;
let cancelScheduled: ReturnType<typeof vi.fn>;
const queued: OutgoingMessage = { to: ["Bob <bob@x.fr>"], cc: [], bcc: [], subject: "Devis", bodyText: "Bonjour", bodyHtml: "<p>Bonjour</p>", references: [], attachments: [], draftId: "d-1" };

beforeEach(() => {
  scheduleSend = vi.fn(() => Promise.resolve(42));
  cancelScheduled = vi.fn(() => Promise.resolve(queued));
  useMail.setState({ backend: { ...demoBackend, demo: false, scheduleSend, cancelScheduled, listScheduled: () => Promise.resolve([]) } as Backend, toasts: [] });
  useComposer.setState({ draft: { ...emptyDraft("a"), to: [{ address: "bob@x.fr", name: null }], subject: "Devis", html: "<p>Bonjour</p>" } });
});

describe("heures proposées", () => {
  const at = (d: string) => scheduleOptions(new Date(d)).map((o) => [o.label.split(" — ")[0], o.at.getDay(), o.at.getHours()]);
  it("dans 1 heure, demain 8 h, puis lundi 8 h", () => {
    expect(at("2026-11-11T14:00:00")).toEqual([["Dans 1 heure", 3, 15], ["Demain matin", 4, 8], ["Lundi matin", 1, 8]]);
  });
  it("pas de lundi en double quand demain est lundi, et le lundi propose le suivant", () => {
    expect(at("2026-11-15T20:00:00").map((x) => x[0])).toEqual(["Dans 1 heure", "Demain matin"]);
    expect(at("2026-11-16T09:00:00").map((x) => x[0])).toEqual(["Dans 1 heure", "Demain matin", "Lundi matin"]);
  });
});

describe("envoi programmé", () => {
  it("met le message en file avec l'heure en secondes, ferme le panneau et propose d'annuler", async () => {
    await useComposer.getState().schedule(1_800_000_123_456);
    expect(scheduleSend).toHaveBeenCalledWith("a", expect.objectContaining({ subject: "Devis" }), 1_800_000_123);
    expect(useComposer.getState().draft).toBeNull();
    expect(useMail.getState().toasts.at(-1)).toMatchObject({ actionLabel: "Annuler" });
  });
  it("refuse sans destinataire, et rappelle la pièce jointe oubliée", async () => {
    useComposer.setState({ draft: { ...emptyDraft("a"), subject: "x", html: "<p>voir pièce jointe</p>" } });
    await useComposer.getState().schedule(Date.now() + 1000);
    expect(useMail.getState().toasts.at(-1)?.message).toContain("destinataire");
    useComposer.setState({ draft: { ...emptyDraft("a"), to: [{ address: "bob@x.fr", name: null }], html: "<p>voir pièce jointe</p>" } });
    await useComposer.getState().schedule(Date.now() + 1000);
    expect(scheduleSend).not.toHaveBeenCalled();
    expect(useMail.getState().toasts.at(-1)).toMatchObject({ actionLabel: "Envoyer quand même" });
  });
  it("garde le panneau ouvert si la programmation échoue", async () => {
    scheduleSend.mockRejectedValueOnce(new Error("heure passée"));
    await useComposer.getState().schedule(Date.now() + 1000);
    expect(useComposer.getState().draft).not.toBeNull();
    expect(useMail.getState().toasts.at(-1)?.message).toContain("heure passée");
  });
  it("modifier un envoi programmé le retire de la file et rouvre le message", async () => {
    await useComposer.getState().editScheduled(42, "a");
    expect(cancelScheduled).toHaveBeenCalledWith(42);
    expect(useComposer.getState().draft).toMatchObject({ id: "d-1", subject: "Devis", accountId: "a" });
    expect(draftFromQueued("a", queued).to.map((r) => r.address)).toEqual(["bob@x.fr"]);
  });
});
