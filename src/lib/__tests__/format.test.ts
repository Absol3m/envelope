import { describe, expect, it } from "vitest";
import { categorize } from "../categories";
import { displayName, fileSize, groupLabel, hue, initials, listDate, relativeTime } from "../format";

const NOW = new Date("2026-11-09T15:00:00");

describe("dates de liste", () => {
  it("heure aujourd'hui, Hier, jour de semaine, puis date", () => {
    expect(listDate("2026-11-09T09:05:00", NOW)).toBe("09:05");
    expect(listDate("2026-11-08T23:59:00", NOW)).toBe("Hier");
    expect(listDate("2026-11-05T10:00:00", NOW)).toMatch(/^jeu/);
    expect(listDate("2026-03-12T10:00:00", NOW)).toMatch(/12 mars/);
    expect(listDate("2025-03-12T10:00:00", NOW)).toMatch(/25/); // année affichée hors année courante
  });
  it("groupes : Aujourd'hui, Hier, Cette semaine, Ce mois-ci, mois", () => {
    expect(groupLabel("2026-11-09T01:00:00", NOW)).toBe("Aujourd'hui");
    expect(groupLabel("2026-11-08T12:00:00", NOW)).toBe("Hier");
    expect(groupLabel("2026-11-04T12:00:00", NOW)).toBe("Cette semaine");
    expect(groupLabel("2026-11-01T12:00:00", NOW)).toBe("Ce mois-ci");
    expect(groupLabel("2026-03-01T12:00:00", NOW)).toBe("Mars 2026");
  });
  it("temps relatif", () => {
    const t = new Date("2026-11-09T14:58:00").toISOString();
    expect(relativeTime(t, NOW.getTime())).toBe("il y a 2 min");
    expect(relativeTime(null)).toBe("jamais");
  });
});

describe("noms et avatars", () => {
  it("nom affichable avec repli sur l'adresse", () => {
    expect(displayName("Alice Martin", "a@x.fr")).toBe("Alice Martin");
    expect(displayName(null, "alice.martin@x.fr")).toBe("alice.martin");
    expect(displayName("  ", null)).toBe("Inconnu");
  });
  it("initiales et teinte stable", () => {
    expect(initials("Camille Laurent")).toBe("CL");
    expect(initials("GitHub")).toBe("G");
    expect(initials("")).toBe("?");
    expect(hue("a@x.fr")).toBe(hue("a@x.fr"));
    expect(hue("a@x.fr")).not.toBe(hue("b@x.fr"));
  });
  it("tailles de fichier", () => {
    expect(fileSize(512)).toBe("512 o");
    expect(fileSize(2048)).toBe("2 Ko");
    expect(fileSize(3_450_000)).toBe("3,3 Mo");
  });
});

describe("Smart Inbox (heuristique)", () => {
  const c = (fromAddr: string, subject = "x", fromName: string | null = null) => categorize({ fromAddr, fromName, subject });
  it("classe notifications, newsletters et personnes", () => {
    expect(c("notifications@github.com")).toBe("notifications");
    expect(c("no-reply@figma.com")).toBe("notifications");
    expect(c("newsletter@lalettretech.fr")).toBe("newsletters");
    expect(c("x@y.fr", "Webinar : -30 % cette semaine")).toBe("newsletters");
    expect(c("camille.laurent@atelier-nord.fr", "Revue de la roadmap")).toBe("priority");
  });
});
