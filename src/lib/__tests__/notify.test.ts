import { describe, expect, it } from "vitest";
import { notificationText } from "../notify";

describe("notification de nouveaux messages", () => {
  it("aucun message : pas de notification", () => {
    expect(notificationText({ fresh: [], freshTotal: 0 })).toBeNull();
  });
  it("un message : expéditeur en titre, objet en corps", () => {
    expect(notificationText({ fresh: [{ from: "Léa", subject: "Photos" }], freshTotal: 1 })).toEqual({ title: "Léa", body: "Photos" });
    expect(notificationText({ fresh: [{ from: "Léa", subject: " " }], freshTotal: 1 })?.body).toBe("(sans objet)");
  });
  it("plusieurs : récapitulatif avec le reste", () => {
    const fresh = [{ from: "A", subject: "x" }, { from: "B", subject: "y" }, { from: "A", subject: "z" }];
    expect(notificationText({ fresh, freshTotal: 5 })).toEqual({ title: "5 nouveaux messages", body: "A, B et 3 autres" });
  });
  it("tronque les textes trop longs", () => {
    expect(notificationText({ fresh: [{ from: "x", subject: "s".repeat(500) }], freshTotal: 1 })?.body.length).toBe(120);
  });
});
