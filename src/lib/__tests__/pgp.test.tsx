// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { PgpBanner } from "@/components/PgpStatus";
import type { DetailMessage, PgpInspection } from "@/lib/api";
import { i18next } from "@/lib/i18n";
import { demoBackend } from "@/lib/mock";

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

  it("la démo n'accepte une clé que pour une adresse qu'elle porte, et refuse de signer avec une clé publique", async () => {
    const pub = await demoBackend.pgpImportKey("-----BEGIN PGP PUBLIC KEY BLOCK-----");
    await expect(demoBackend.pgpSetAcceptance(pub.fingerprint, "eve@example.com", "verified")).rejects.toThrow();
    expect(await demoBackend.pgpRecipientKey("demo@example.com")).toBeNull();
    await demoBackend.pgpSetAcceptance(pub.fingerprint, "Demo@example.com", "unverified");
    expect(await demoBackend.pgpRecipientKey("demo@example.com")).toBe(pub.fingerprint);
    await demoBackend.pgpSetAcceptance(pub.fingerprint, "demo@example.com", "rejected");
    expect(await demoBackend.pgpRecipientKey("demo@example.com")).toBeNull();
    await expect(demoBackend.pgpSetSigningKey("pro", "alex@atelier-nord.fr", pub.fingerprint)).rejects.toThrow();
    await demoBackend.pgpDeleteKey(pub.fingerprint);
  });
});
