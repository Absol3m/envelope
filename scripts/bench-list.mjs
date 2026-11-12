// Mesure de la liste des conversations dans Edge (temps réel, sans dépendance) : nombre de nœuds DOM, tas JS,
// durée du défilement jusqu'au bout, et retour de la navigation clavier vers une ligne lointaine.
//   pnpm dev --port 5199   (autre terminal)   puis   node scripts/bench-list.mjs 10000
// Le mode `?bench=N` n'existe qu'en `pnpm dev` (voir src/dev.ts). Il faut Microsoft Edge ou adapter `edge` ci-dessous.
import { spawn } from "node:child_process";
const n = process.argv[2] ?? "1000";
const edge = process.env.EDGE ?? "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";
const port = 9300 + Math.floor(Math.random() * 500);
const proc = spawn(edge, ["--headless=new", "--disable-gpu", `--remote-debugging-port=${port}`, "--window-size=1400,900", "--enable-precise-memory-info", "--user-data-dir=" + (process.env.TMPDIR ?? "/tmp") + "/edge-bench-" + port, "about:blank"], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let tabs;
for (let i = 0; i < 60; i++) { try { tabs = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); if (tabs.length) break; } catch {} await sleep(250); }
const ws = new WebSocket(tabs.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d.result); pending.delete(d.id); } };
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
await send("Page.enable"); await send("Page.navigate", { url: `http://localhost:5199/?bench=${n}` });
let out = "";
for (let i = 0; i < 240; i++) {
  await sleep(500);
  const r = await send("Runtime.evaluate", { expression: "document.getElementById('bench')?.textContent ?? ''", returnByValue: true });
  out = r?.result?.value ?? "";
  if (out.startsWith("{")) break;
}
console.log(out || "(pas de résultat)");
proc.kill();
process.exit(0);
