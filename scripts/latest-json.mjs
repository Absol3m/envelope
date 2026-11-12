#!/usr/bin/env node
// Manifeste de mise à jour (`latest.json`, format de tauri-plugin-updater) d'une Release signée :
//   node scripts/latest-json.mjs <étiquette> <dossier des fichiers et de leurs .sig> > latest.json
// Chaque plateforme pointe sur le fichier attaché à la Release de l'étiquette et porte sa signature (contenu du .sig).
// Échoue si une plateforme manque : publier un canal partiel laisserait des installations sans mise à jour.
import { readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";

const [tag, dir] = process.argv.slice(2);
if (!tag || !dir) {
  console.error("usage : latest-json.mjs <étiquette> <dossier>");
  process.exit(2);
}
const repo = process.env.GITHUB_REPOSITORY ?? "Absol3m/envelope";

/** Plateforme (clé de l'updater) → fichier installé par la mise à jour. */
const PLATFORMS = {
  "windows-x86_64": (f) => f.endsWith("-setup.exe"),
  "darwin-aarch64": (f) => f.endsWith(".app.tar.gz"),
  "linux-x86_64": (f) => f.endsWith(".AppImage"),
};

const files = readdirSync(dir, { recursive: true, withFileTypes: true })
  .filter((e) => e.isFile())
  .map((e) => join(e.parentPath, e.name));

const platforms = {};
for (const [platform, matches] of Object.entries(PLATFORMS)) {
  const found = files.filter((f) => matches(basename(f)));
  if (found.length !== 1) {
    console.error(`${platform} : ${found.length} fichier(s) trouvé(s), 1 attendu`);
    process.exit(1);
  }
  const [file] = found;
  const sig = files.find((f) => basename(f) === `${basename(file)}.sig`);
  if (!sig) {
    console.error(`${platform} : signature ${basename(file)}.sig introuvable`);
    process.exit(1);
  }
  platforms[platform] = {
    signature: readFileSync(sig, "utf8").trim(),
    url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(basename(file))}`,
  };
}

const manifest = {
  version: tag.replace(/^v/, ""),
  notes: `Notes de version : https://github.com/${repo}/releases/tag/${tag}`,
  pub_date: new Date().toISOString(),
  platforms,
};
process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
