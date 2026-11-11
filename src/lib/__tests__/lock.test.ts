// @vitest-environment jsdom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, LockStatus } from "../api";
import { useAutoLock } from "../autolock";
import { notificationText } from "../notify";
import { demoBackend } from "../mock";
import { useMail } from "../store";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const status = (over: Partial<LockStatus> = {}): LockStatus => ({ method: "password", locked: false, timeoutSecs: 300, systemAvailable: true, ...over });
let lockNow: ReturnType<typeof vi.fn>;

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

/** Monte un composant qui n'appelle que le crochet. */
async function mountAutoLock(secs: number, active: boolean) {
  const Host = () => { useAutoLock(secs, active); return null; };
  await act(async () => { createRoot(document.body.appendChild(document.createElement("div"))).render(createElement(Host)); });
}

beforeEach(() => {
  lockNow = vi.fn(() => Promise.resolve());
  useMail.setState({ backend: { ...demoBackend, lockNow } as Backend, lock: status() });
});
afterEach(() => { vi.useRealTimers(); document.body.innerHTML = ""; });

describe("verrouillage", () => {
  it("verrouiller efface les données affichées et passe l'état à « verrouillé »", async () => {
    useMail.setState({ accounts: [{ id: "a" } as never], threads: [{ id: 1 } as never], details: { 1: {} as never }, selectedId: 1 });
    await useMail.getState().lockNow();
    const s = useMail.getState();
    expect(lockNow).toHaveBeenCalledOnce();
    expect([s.lock?.locked, s.accounts, s.threads, s.details, s.selectedId]).toEqual([true, [], [], {}, null]);
  });

  it("la notification d'un message n'affiche ni expéditeur ni objet quand l'application est verrouillée", () => {
    const p = { freshTotal: 1, fresh: [{ from: "Alice <a@x.fr>", subject: "Salaire" }] };
    expect(notificationText(p)).toEqual({ title: "Alice <a@x.fr>", body: "Salaire" });
    const hidden = notificationText(p, true)!;
    expect(JSON.stringify(hidden)).not.toMatch(/Alice|Salaire/);
    expect(notificationText({ freshTotal: 3, fresh: p.fresh }, true)!.body).toBe("3 nouveaux messages");
    // Le moteur n'envoie aucun aperçu verrouillé : la notification générique doit quand même apparaître.
    expect(notificationText({ freshTotal: 2, fresh: [] }, true)).toEqual({ title: "Envelope", body: "2 nouveaux messages" });
    expect(notificationText({ freshTotal: 2, fresh: [] })).toBeNull();
  });
});

describe("verrouillage automatique", () => {
  it("verrouille après le délai sans activité, pas avant, et l'activité repousse l'échéance", async () => {
    vi.useFakeTimers();
    await mountAutoLock(60, true);
    await vi.advanceTimersByTimeAsync(50_000);
    expect(lockNow).not.toHaveBeenCalled();
    window.dispatchEvent(new Event("keydown")); // activité à t = 50 s
    await vi.advanceTimersByTimeAsync(40_000); // t = 90 s : 40 s d'inactivité seulement
    expect(lockNow).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(30_000); // t = 120 s : 70 s d'inactivité
    expect(lockNow).toHaveBeenCalled();
  });

  it("ne fait rien si le délai est 0 ou si le verrouillage est inactif", async () => {
    vi.useFakeTimers();
    await mountAutoLock(0, true);
    await mountAutoLock(60, false);
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(lockNow).not.toHaveBeenCalled();
  });
});
