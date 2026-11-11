// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { applyFont, customName, FREE_FONTS, fontStack, parseFont, SYSTEM_STACK } from "../font";

describe("police de l'interface", () => {
  it("par défaut : la police du système, sans variante typographique", () => {
    expect(fontStack("system")).toEqual({ stack: SYSTEM_STACK, features: "normal" });
    expect(parseFont("n'importe quoi")).toBe("system");
    expect(SYSTEM_STACK.startsWith("system-ui")).toBe(true);
  });
  it("polices libres : la police choisie d'abord, celle du système en repli", () => {
    for (const f of FREE_FONTS) expect(fontStack(f.id).stack).toBe(`${f.family}, ${SYSTEM_STACK}`);
    expect(fontStack("inter").features).toContain("cv11"); // jeux stylistiques propres à Inter seulement
    expect(fontStack("source-sans").features).toBe("normal");
    expect(parseFont("plex-sans")).toBe("plex-sans");
  });
  it("police installée : nom validé, jamais de caractère qui sorte des guillemets CSS", () => {
    expect(customName("  Helvetica   Neue ")).toBe("Helvetica Neue");
    expect(customName("SF Pro-Text 2.0")).toBe("SF Pro-Text 2.0");
    expect(customName("Noto Sans Français")).toBe("Noto Sans Français");
    for (const bad of ["", "   ", 'x"; } body { display:none', "a;b", "url(x)", "a\\b", "a'b", "x".repeat(61), "{}"]) expect(customName(bad)).toBeNull();
    expect(fontStack("custom:Helvetica Neue").stack).toBe(`"Helvetica Neue", ${SYSTEM_STACK}`);
    // Un réglage corrompu ne casse rien : retour à la police du système.
    expect(parseFont('custom:a"b')).toBe("system");
    expect(fontStack('custom:a"b' as never).stack).toBe(SYSTEM_STACK);
  });
  it("applique la pile et les variantes sur l'élément racine", async () => {
    const root = document.createElement("div");
    await applyFont("custom:Georgia", root);
    expect(root.style.getPropertyValue("--font-sans")).toBe(`"Georgia", ${SYSTEM_STACK}`);
    expect(root.style.getPropertyValue("--font-features")).toBe("normal");
    await applyFont("system", root);
    expect(root.style.getPropertyValue("--font-sans")).toBe(SYSTEM_STACK);
  });
});
