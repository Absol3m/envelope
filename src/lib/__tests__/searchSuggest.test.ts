import { describe, expect, it } from "vitest";
import { applySuggestion, contextAt, operatorItems, tokenAt, valueItems } from "../searchSuggest";

describe("suggestions de recherche", () => {
  it("repère le mot sous le curseur", () => {
    expect(tokenAt("budget de:al", 12)).toEqual({ start: 7, end: 12, text: "de:al" });
    expect(tokenAt("budget ", 7)).toEqual({ start: 7, end: 7, text: "" });
  });
  it("opérateurs : filtre sans tenir compte des accents", () => {
    expect(operatorItems("").length).toBe(9);
    expect(operatorItems("a").map((i) => i.label)).toEqual(["à:", "avant:", "apres:"]);
    expect(operatorItems("non")[0]).toMatchObject({ label: "non-lu", insert: "non-lu " });
    expect(operatorItems("zzz")).toEqual([]);
  });
  it("contexte : opérateur, valeur de contact, valeur de date, mot inconnu", () => {
    expect(contextAt("de", 2)).toMatchObject({ kind: "operator", prefix: "de" });
    expect(contextAt("de:ali", 6)).toMatchObject({ kind: "contact", op: "de", prefix: "ali" });
    expect(contextAt("À:léa", 5)).toMatchObject({ kind: "contact", op: "a", prefix: "léa" });
    expect(contextAt("avant:", 6)).toMatchObject({ kind: "value", op: "avant", prefix: "" });
    expect(contextAt("12:30", 5)).toBeNull();
  });
  it("dates : raccourcis relatifs au jour courant", () => {
    const now = new Date(2026, 0, 15);
    const v = valueItems("avant", "", now);
    expect(v.map((i) => i.insert)).toEqual(["2026-01-15 ", "2026-01 ", "2025-12 ", "2026 ", "2025 "]);
    expect(valueItems("avant", "2025", now).map((i) => i.insert)).toEqual(["2025-12 ", "2025 "]);
    expect(valueItems("is", "un", now)).toHaveLength(1);
  });
  it("dossiers : valeurs de dans:", () => {
    expect(valueItems("dans", "arch").map((i) => i.insert)).toEqual(["archives "]);
    expect(valueItems("dans", "").length).toBe(7);
    expect(contextAt("dans:co", 7)).toMatchObject({ kind: "value", op: "dans", prefix: "co" });
  });
  it("insère à la place du mot en cours", () => {
    const q = "budget de:al reste";
    const token = tokenAt(q, 12);
    expect(applySuggestion(q, token, "de:", "alice@x.fr ")).toEqual({ text: "budget de:alice@x.fr reste", caret: 20 });
    expect(applySuggestion("bud", tokenAt("bud", 3), "", "de:")).toEqual({ text: "de:", caret: 3 });
    expect(applySuggestion("bud", tokenAt("bud", 3), "", "pj ")).toEqual({ text: "pj ", caret: 3 });
  });
});
