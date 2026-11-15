// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { PgpBanner } from "@/components/PgpStatus";
import type { DetailMessage, PgpInspection } from "@/lib/api";
import { i18next } from "@/lib/i18n";

beforeAll(async () => {
  await i18next.changeLanguage("en");
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});
afterEach(() => { document.body.innerHTML = ""; });

const msg = (): DetailMessage => ({ id: 1, bodyText: "bonjour", oversize: false } as DetailMessage);
const result = (over: Partial<PgpInspection>): PgpInspection => ({ encrypted: false, error: null, signature: null, bodyText: null, bodyHtml: null, ...over });

async function banner(r: PgpInspection | null, unlock = vi.fn()) {
  const host = document.body.appendChild(document.createElement("div"));
  await act(async () => { createRoot(host).render(createElement(PgpBanner, { pgp: { result: r, busy: false, tried: false, unlock, shown: msg() } })); });
  return host;
}

describe("PGP", () => {
  it("affiche le verdict de signature", async () => {
    const sig = (state: "valid" | "invalid" | "unknown_key") => result({ signature: { state, signer: "Alice <a@x.fr>", fingerprint: "AB" } });
    expect((await banner(sig("valid"))).textContent).toMatch(/Signature verified.*Alice/);
    document.body.innerHTML = "";
    expect((await banner(sig("invalid"))).textContent).toMatch(/Invalid signature/);
    document.body.innerHTML = "";
    expect((await banner(sig("unknown_key"))).textContent).toMatch(/haven't imported/);
  });

  it("demande la phrase secrète d'une clé verrouillée et la transmet", async () => {
    const unlock = vi.fn();
    const host = await banner(result({ encrypted: true, error: "passphrase" }), unlock);
    const input = host.querySelector<HTMLInputElement>('input[type="password"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "sésame");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    expect(unlock).toHaveBeenCalledWith("sésame");
  });

  it("n'affiche rien pour un message qui n'est pas du PGP", async () => {
    expect((await banner(null)).textContent).toBe("");
  });
});
