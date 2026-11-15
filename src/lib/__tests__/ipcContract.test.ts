// Contrat des commandes Tauri : les noms et les clés d'arguments des `invoke(...)` écrits à la main doivent correspondre aux
// commandes enregistrées (`generate_handler!`) et aux paramètres Rust (camelCase). Un paramètre renommé côté Rust compile des
// deux côtés et ne casse qu'à l'exécution : ce test le fait échouer.
// Analyseur maison (regex), faute de mieux : tauri-specta remplacerait les ~65 `invoke` écrits à la main et le
// registre des commandes (changement de contrat plus lourd, voir #98) ; ici le périmètre est borné et testé par mutation
// (renommer une clé dans api.ts fait échouer le test). Il ignore `rename_all`, `#[serde(default)]` et les signatures
// sur plusieurs lignes avec `where` : aucune commande n'en utilise.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const root = join(__dirname, "../../..");
const read = (p: string) => readFileSync(join(root, p), "utf8");
const walk = (dir: string, ext: string): string[] =>
  (readdirSync(join(root, dir), { recursive: true }) as string[]).filter((f) => f.endsWith(ext)).map((f) => `${dir}/${f}`);

const camel = (s: string) => s.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());

/** Commandes enregistrées : `module::nom,` dans `generate_handler![ … ]`. */
const handlers = (): string[] => {
  const src = read("src-tauri/src/lib.rs");
  const body = src.slice(src.indexOf("generate_handler!["));
  return [...body.slice(0, body.indexOf("])")).matchAll(/\b[a-z_]+::([a-z_0-9]+),/g)].map((m) => m[1]);
};

/** Paramètres (hors injection Tauri) d'une commande : nom camelCase → obligatoire ? */
const params = (rust: string, name: string): Map<string, boolean> | undefined => {
  const m = new RegExp(`pub (?:async )?fn ${name}\\s*(?:<[^>]*>)?\\(([^)]*(?:\\([^)]*\\)[^)]*)*)\\)`).exec(rust);
  if (!m) return undefined;
  const out = new Map<string, boolean>();
  for (const raw of splitTop(m[1], '"')) {
    const p = /^\s*(?:mut\s+)?([a-z_0-9]+)\s*:\s*([\s\S]+?)\s*$/.exec(raw);
    if (!p || /\b(State|AppHandle|Window|WebviewWindow|Webview)\b/.test(p[2])) continue;
    out.set(camel(p[1]), !/^Option</.test(p[2]));
  }
  return out;
};

/** Découpe sur les virgules du premier niveau (ni dans (), [], {}, <> ni dans une chaîne). */
function splitTop(s: string, quotes = "\"'`"): string[] {
  const parts: string[] = [];
  let depth = 0, cur = "", quote = "";
  for (const c of s) {
    if (quote) { cur += c; if (c === quote) quote = ""; continue; }
    if (quotes.includes(c)) { quote = c; cur += c; continue; }
    if ("([{<".includes(c)) depth++;
    if (")]}>".includes(c) && !(c === ">" && cur.endsWith("=")) ) depth--;
    if (c === "," && depth === 0) { parts.push(cur); cur = ""; } else cur += c;
  }
  if (cur.trim()) parts.push(cur);
  return parts;
}

const CALLERS = ["src/lib/api.ts", "src/lib/utils.ts", "src/lib/update.ts"];

/** Appels `invoke("cmd", { clés })` du front : commande → clés d'arguments (null : aucun argument). */
const calls = (): { cmd: string; keys: string[]; file: string }[] => {
  const out: { cmd: string; keys: string[]; file: string }[] = [];
  for (const file of CALLERS) {
    const src = read(file);
    for (const m of src.matchAll(/\binvoke(?:<[^>(]*(?:<[^>]*>)?[^>(]*>)?\(\s*"([a-z_0-9:|-]+)"\s*,\s*\{/g)) {
      let i = m.index! + m[0].length, depth = 1;
      const start = i;
      while (i < src.length && depth > 0) { if (src[i] === "{") depth++; if (src[i] === "}") depth--; i++; }
      const keys = splitTop(src.slice(start, i - 1)).map((e) => /^\s*([A-Za-z_0-9]+)/.exec(e)?.[1] ?? "").filter(Boolean);
      out.push({ cmd: m[1], keys, file });
    }
    for (const m of src.matchAll(/\binvoke(?:<[^>(]*(?:<[^>]*>)?[^>(]*>)?\(\s*"([a-z_0-9:|-]+)"\s*\)/g)) out.push({ cmd: m[1], keys: [], file });
  }
  return out.filter((c) => !c.cmd.includes(":"));
};

describe("contrat IPC : commandes Tauri appelées par le front", () => {
  const known = new Set(handlers());
  const allCalls = calls();
  const rust = walk("src-tauri/src", ".rs").map(read).join("\n");

  it("trouve les commandes et les appels", () => {
    expect(known.size).toBeGreaterThan(50);
    expect(allCalls.length).toBeGreaterThan(50);
  });

  it("aucun autre fichier du front n'appelle `invoke` hors de ceux analysés", () => {
    const others = walk("src", ".ts").concat(walk("src", ".tsx"))
      .filter((f) => !f.includes("__tests__") && !f.includes("/bindings/") && !CALLERS.includes(f) && /\binvoke\(/.test(read(f)));
    expect(others).toEqual([]);
  });

  it("chaque commande appelée est enregistrée", () => {
    const unknown = allCalls.filter((c) => !known.has(c.cmd)).map((c) => c.cmd);
    expect(unknown).toEqual([]);
  });

  it("les clés d'arguments correspondent aux paramètres Rust (camelCase)", () => {
    const errors: string[] = [];
    for (const c of allCalls) {
      const p = params(rust, c.cmd);
      if (!p) { errors.push(`${c.cmd} : définition Rust introuvable`); continue; }
      for (const k of c.keys) if (!p.has(k)) errors.push(`${c.cmd} : clé « ${k} » inconnue (attendu : ${[...p.keys()].join(", ") || "aucun"})`);
      for (const [k, required] of p) if (required && !c.keys.includes(k)) errors.push(`${c.cmd} : paramètre « ${k} » manquant`);
    }
    expect(errors).toEqual([]);
  });
});
