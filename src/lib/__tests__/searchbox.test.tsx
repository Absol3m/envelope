// @vitest-environment jsdom
import { createElement, useState, act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { SearchBox } from "@/components/SearchBox";
import type { Backend } from "../api";

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});
afterEach(() => { document.body.innerHTML = ""; });

const contacts = [{ name: "Alice Martin", address: "alice@x.fr" }];
const backend = { suggestContacts: vi.fn(async () => contacts) } as unknown as Backend;

async function mount(initial = "") {
  const seen = { query: initial };
  function Host() {
    const [q, setQ] = useState(initial);
    seen.query = q;
    return createElement(SearchBox, { query: q, setQuery: setQ, backend, accountId: null });
  }
  const host = document.body.appendChild(document.createElement("div"));
  await act(async () => { createRoot(host).render(createElement(Host)); });
  const el = host.querySelector("input")!;
  const type = async (value: string) => {
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(el, value);
      el.setSelectionRange(value.length, value.length);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const key = async (k: string, init: KeyboardEventInit = {}) => {
    const ev = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init });
    await act(async () => { el.dispatchEvent(ev); });
    return ev;
  };
  await act(async () => { el.focus(); });
  return { el, seen, type, key, options: () => [...document.querySelectorAll('[role="option"]')].map((o) => o.textContent) };
}

describe("barre de recherche : suggestions", () => {
  it("propose les opérateurs et Tab complète avec le premier", async () => {
    const s = await mount();
    expect(s.options()[0]).toContain("de:");
    const ev = await s.key("Tab");
    expect(ev.defaultPrevented).toBe(true);
    expect(s.seen.query).toBe("de:");
  });

  it("filtre à la frappe et complète un opérateur sans valeur", async () => {
    const s = await mount();
    await s.type("obj");
    expect(s.options()).toHaveLength(1);
    await s.key("Tab");
    expect(s.seen.query).toBe("objet:");
  });

  it("propose les contacts après de: et Tab insère l'adresse", async () => {
    const s = await mount();
    await s.type("de:ali");
    await act(async () => { await new Promise((r) => setTimeout(r, 200)); });
    expect(s.options()[0]).toContain("Alice Martin");
    await s.key("Tab");
    expect(s.seen.query).toBe("de:alice@x.fr ");
  });

  it("flèches puis Entrée choisissent ; Entrée seule ne complète pas ; Échap ferme le menu sans effacer", async () => {
    const s = await mount();
    const enter = await s.key("Enter");
    expect(enter.defaultPrevented).toBe(false);
    await s.key("ArrowDown");
    await s.key("Enter");
    expect(s.seen.query).toBe("à:");
    await s.type("budget ");
    expect(s.options().length).toBeGreaterThan(0);
    await s.key("Escape");
    expect(s.options()).toHaveLength(0);
    expect(s.seen.query).toBe("budget ");
  });

  it("sans suggestion, Tab garde son rôle (changer de champ)", async () => {
    const s = await mount();
    await s.type("budget");
    expect(s.options()).toHaveLength(0);
    expect((await s.key("Tab")).defaultPrevented).toBe(false);
  });
});
