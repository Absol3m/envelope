// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ThreadList } from "@/components/ThreadList";
import { TooltipProvider } from "@/components/ui/primitives";
import type { ThreadSummary } from "../api";
import { demoBackend } from "../mock";
import { useMail } from "../store";

const VIEWPORT = 600;
const ROW = 54;

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom n'a pas de mise en page : on lui donne une fenêtre de 600 px et des lignes de 54 px.
  class RO { constructor(private cb: ResizeObserverCallback) {} observe(el: Element) { this.cb([{ target: el, contentRect: { width: 400, height: VIEWPORT } } as ResizeObserverEntry], this as unknown as ResizeObserver); } unobserve() {} disconnect() {} }
  vi.stubGlobal("ResizeObserver", RO);
  (document.defaultView as unknown as { ResizeObserver: typeof RO }).ResizeObserver = RO; // la bibliothèque lit celui de la fenêtre du document
  Element.prototype.getBoundingClientRect = function () { return { width: 400, height: (this as HTMLElement).dataset.index ? ROW : VIEWPORT, top: 0, left: 0, right: 400, bottom: 0, x: 0, y: 0, toJSON() {} } as DOMRect; };
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get() { return (this as HTMLElement).dataset.index ? ROW : VIEWPORT; } });
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, get() { return 400; } });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get() { return VIEWPORT; } });
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get() { return 400; } });
  Element.prototype.scrollTo = function () {};
});
afterEach(() => { document.body.innerHTML = ""; });

const summary = (i: number): ThreadSummary => ({ id: i + 1, accountId: "a", subject: `Objet ${i + 1}`, snippet: "aperçu", lastMessageAt: new Date(Date.UTC(2026, 0, 1) - i * 3_600_000).toISOString(), messageCount: 1, unreadCount: 0, isStarred: false, hasAttachments: false, fromName: "Zoe", fromAddr: "zoe@x.fr", participants: null, folderId: 1, reminderKind: null, reminderDue: null, reminderFired: null });

describe("liste des conversations", () => {
  it("ne met dans le DOM que les lignes visibles, quel que soit le nombre de conversations chargées", async () => {
    useMail.setState({ backend: demoBackend, threads: Array.from({ length: 3000 }, (_, i) => summary(i)), hasMore: false, threadsLoading: false, ready: true, view: { kind: "role", role: "inbox" }, query: "", category: "all" });
    const host = document.body.appendChild(document.createElement("div"));
    await act(async () => { createRoot(host).render(createElement(TooltipProvider, null, createElement(ThreadList))); });
    await act(async () => { await new Promise((r) => setTimeout(r, 50)); }); // le fenêtrage se calcule après la mesure du conteneur
    const options = host.querySelectorAll('[role="option"]');
    expect(options.length).toBeGreaterThan(5);
    expect(options.length).toBeLessThan(60); // 3000 conversations chargées : une quarantaine de lignes dans le DOM, pas 3000
    // Les lecteurs d'écran connaissent la taille réelle de la liste et la position de chaque ligne.
    expect(options[0].getAttribute("aria-setsize")).toBe("3000");
    expect(options[0].getAttribute("aria-posinset")).toBe("1");
    expect(host.textContent).toContain("Objet 1");
    expect(host.textContent).not.toContain("Objet 2999");
  });
});
