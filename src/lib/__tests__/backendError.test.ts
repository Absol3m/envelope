import { describe, expect, it } from "vitest";
import { BackendError, describeStored, describeWire, toBackendError } from "../backendError";
import { i18next } from "../i18n";

const wire = (code: string, args: Record<string, string> = {}, message = "fallback") => ({ code, args, message });

describe("erreurs du moteur", () => {
  it("le code est traduit avec ses paramètres, dans la langue active", async () => {
    const e = wire("lock.too_many_attempts", { seconds: "12" });
    expect(describeWire(e)).toBe("Trop d'essais : réessayez dans 12 s");
    await i18next.changeLanguage("en");
    try {
      expect(describeWire(e)).toBe("Too many attempts: try again in 12 s");
    } finally {
      await i18next.changeLanguage("fr");
    }
  });
  it("pluriel selon count", () => {
    expect(describeWire(wire("local.folder_not_empty", { count: "1" }))).toBe("Ce dossier contient 1 message : videz-le d'abord");
    expect(describeWire(wire("local.folder_not_empty", { count: "3" }))).toBe("Ce dossier contient 3 messages : videz-le d'abord");
  });
  it("texte brut ou code inconnu : le message du moteur", () => {
    expect(describeWire(wire("", {}, "connection reset by peer"))).toBe("connection reset by peer");
    expect(describeWire(wire("code.inexistant", {}, "message de repli"))).toBe("message de repli");
  });
  it("catégories de bibliothèques avec leur détail", () => {
    expect(describeWire(wire("lib.io", { detail: "disque plein" }))).toBe("Entrée/sortie : disque plein");
  });
  it("une erreur rejetée par invoke devient une BackendError lisible par String()", () => {
    const e = toBackendError(wire("lock.wrong_password"));
    expect(e).toBeInstanceOf(BackendError);
    expect(String(e)).toBe("Mot de passe incorrect");
    expect(toBackendError("texte")).toBe("texte"); // erreur d'un plugin : inchangée
  });
  it("erreur gardée en base : JSON traduit, texte ordinaire inchangé", () => {
    expect(describeStored(JSON.stringify(wire("lock.wrong_password")))).toBe("Mot de passe incorrect");
    expect(describeStored("ancien message en clair")).toBe("ancien message en clair");
    expect(describeStored(null)).toBeNull();
    expect(describeStored("{pas du json")).toBe("{pas du json");
  });
});
