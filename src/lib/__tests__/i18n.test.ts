import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/** Vérifie les fichiers de langue : mêmes clés et mêmes variables en `fr` et `en`, et aucune clé utilisée dans le code qui n'existe pas. */
const ROOT = join(__dirname, "../../locales");
const SRC = join(__dirname, "../..");

type Tree = { [k: string]: string | Tree };
const load = (lang: string) => Object.fromEntries(readdirSync(join(ROOT, lang)).filter((f) => f.endsWith(".json")).map((f) => [f.replace(/\.json$/, ""), JSON.parse(readFileSync(join(ROOT, lang, f), "utf8")) as Tree]));
const flatten = (t: Tree, prefix = ""): Record<string, string> => Object.entries(t).reduce<Record<string, string>>((acc, [k, v]) => (typeof v === "string" ? { ...acc, [prefix + k]: v } : { ...acc, ...flatten(v, `${prefix}${k}.`) }), {});
const vars = (s: string) => [...s.matchAll(/\{\{\s*(\w+)\s*\}\}/g)].map((m) => m[1]).sort();
/** Un pluriel i18next s'écrit `clé_one` / `clé_other` : la clé « logique » est sans le suffixe. */
const base = (k: string) => k.replace(/_(one|other)$/, "");

const en = load("en");
const fr = load("fr");
const flat = (all: Record<string, Tree>) => Object.fromEntries(Object.entries(all).flatMap(([ns, t]) => Object.entries(flatten(t)).map(([k, v]) => [`${ns}:${k}`, v])));
const enKeys = flat(en);
const frKeys = flat(fr);

const sig = (s: string) => vars(s).filter((v) => v !== "count").join();
describe("fichiers de langue", () => {
  it("mêmes espaces de noms et mêmes clés en français et en anglais", () => {
    expect(Object.keys(fr).sort()).toEqual(Object.keys(en).sort());
    const logical = (m: Record<string, string>) => [...new Set(Object.keys(m).map(base))].sort();
    expect(logical(frKeys)).toEqual(logical(enKeys));
  });
  it("mêmes variables d'interpolation dans chaque langue", () => {
    const diffs = Object.keys(frKeys).filter((k) => enKeys[k] !== undefined && vars(frKeys[k]).join() !== vars(enKeys[k]).join());
    // `count` peut manquer d'un côté d'un pluriel (« une seule conversation ») : on compare hors `count`.
    const real = diffs.filter((k) => sig(frKeys[k]) !== sig(enKeys[k]));
    expect(real).toEqual([]);
  });
  it("toute langue ajoutée (hors fr) : les textes présents ont les mêmes variables que l'anglais", () => {
    const others = readdirSync(ROOT).filter((l) => l !== "en" && l !== "fr" && statSync(join(ROOT, l)).isDirectory());
    for (const lang of others) {
      const keys = flat(load(lang));
      const bad = Object.keys(keys).filter((k) => enKeys[k] !== undefined && sig(keys[k]) !== sig(enKeys[k]));
      expect(bad, lang).toEqual([]);
    }
  });
  it("le contexte des textes ambigus ne désigne que des textes qui existent", () => {
    const ctx = JSON.parse(readFileSync(join(SRC, "../docs/translations/context.json"), "utf8")) as Record<string, string>;
    expect(Object.keys(ctx).filter((k) => !(k in enKeys) && !(`${k}_one` in enKeys))).toEqual([]);
  });
  it("aucune valeur vide", () => {
    expect(Object.entries({ ...enKeys, ...frKeys }).filter(([, v]) => v.trim() === "").map(([k]) => k)).toEqual([]);
  });
  it("chaque clé écrite en toutes lettres dans le code existe", () => {
    const files: string[] = [];
    const walk = (d: string) => readdirSync(d).forEach((n) => { const p = join(d, n); if (n === "__tests__" || n === "locales") return; statSync(p).isDirectory() ? walk(p) : /\.(tsx?|ts)$/.test(n) && files.push(p); });
    walk(SRC);
    const known = new Set(Object.keys(enKeys).map(base));
    const missing: string[] = [];
    for (const f of files) {
      for (const m of readFileSync(f, "utf8").matchAll(/\b(?:tr|t)\(\s*["'`]([a-z][A-Za-z]*):([A-Za-z0-9_.-]+)["'`]/g)) if (!known.has(`${m[1]}:${m[2]}`)) missing.push(`${f.replace(SRC, "src")} → ${m[1]}:${m[2]}`);
    }
    expect(missing).toEqual([]);
  });
  it("la boîte de démonstration n'affiche aucune clé brute, en français comme en anglais", async () => {
    const { i18next } = await import("../i18n");
    const { demoBackend } = await import("../mock");
    for (const lang of ["fr", "en"]) {
      await i18next.changeLanguage(lang); // reconstruit le jeu de données
      const threads = await demoBackend.listThreads({ limit: 1000 });
      const texts = (await Promise.all(threads.map((t) => demoBackend.getThread(t.id)))).flatMap((d) => d.messages.flatMap((m) => [m.subject, m.fromName ?? "", m.bodyText ?? "", m.bodyHtml ?? ""]));
      texts.push(...(await demoBackend.listTemplates()).flatMap((t) => [t.name, t.subject, t.bodyHtml]));
      expect(texts.filter((s) => /^mail\.|\{\{/.test(s)), lang).toEqual([]);
    }
    await i18next.changeLanguage("fr");
  });
  it("chaque code d'erreur du moteur Rust (msg!) a sa traduction", () => {
    const rust: string[] = [];
    const walk = (d: string) => readdirSync(d).forEach((n) => { const p = join(d, n); statSync(p).isDirectory() ? walk(p) : n.endsWith(".rs") && rust.push(p); });
    walk(join(SRC, "../src-tauri/src"));
    const known = new Set(Object.keys(enKeys).map(base));
    const missing: string[] = [];
    for (const f of rust) for (const m of readFileSync(f, "utf8").matchAll(/msg!\(\s*"([a-z_.]+)"/g)) if (!known.has(`errors:rust.${m[1]}`)) missing.push(`${f.split("src-tauri/")[1]} → ${m[1]}`);
    expect(missing).toEqual([]);
  });
});
