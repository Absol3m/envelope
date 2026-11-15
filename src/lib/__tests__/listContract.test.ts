import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { DetailMessage, ThreadQuery } from "../api";
import { demoBackend, listThreads, setDemoThreads, type MockThread } from "../mock";

/** Contrat de la liste : même scénario que le moteur Rust (src-tauri/src/mail/list_contract.rs), mêmes réponses attendues. */
interface Scenario {
  folders: { id: number; account: string; role: MockThread["role"] }[];
  threads: { id: number; account: string; folder: number; starred: boolean; attachments: boolean; reminder?: MockThread["reminder"]; messages: { from: [string, string]; subject: string; date: string; read: boolean; text: string; folder?: number; to?: [string | null, string][]; cc?: [string | null, string][] }[] }[];
  queries: { name: string; query: ThreadQuery; ids: number[] }[];
  summaries: Record<string, Record<string, unknown>>;
}
const scenario = JSON.parse(readFileSync(join(__dirname, "../../../src-tauri/tests/contract/list_threads.json"), "utf8")) as Scenario;

let nextId = 1;
const demoThreads = (): MockThread[] =>
  scenario.threads.map((t) => {
    const folderId = t.folder;
    const messages: DetailMessage[] = t.messages.map((m) => ({
      id: nextId++, messageId: null, references: [], folderId: m.folder ?? folderId, subject: m.subject,
      fromName: m.from[0], fromAddr: m.from[1], to: (m.to ?? []).map(([name, address]) => ({ name, address })), cc: (m.cc ?? []).map(([name, address]) => ({ name, address })), date: m.date, bodyText: m.text, bodyHtml: null,
      isRead: m.read, isStarred: false, unsubscribe: null, authFail: null, oversize: false, size: null,
    }));
    return {
      id: t.id, accountId: t.account, role: scenario.folders.find((f) => f.id === folderId)!.role, starred: t.starred, messages,
      attachments: t.attachments ? [{ id: t.id, messageId: messages[messages.length - 1].id, filename: "pj.pdf", mimeType: "application/pdf", size: 1, contentId: null, isInline: false, previewable: false }] : [],
      reminder: t.reminder,
    };
  });

describe("liste des conversations : contrat moteur / démo", () => {
  setDemoThreads(demoThreads());

  for (const q of scenario.queries) {
    it(q.name, () => {
      expect(listThreads(q.query).map((t) => t.id)).toEqual(q.ids);
    });
  }

  it("résumés : tous les champs, pour toutes les conversations", () => {
    const all = [...listThreads({ limit: 500 }), ...listThreads({ remindersOnly: true })]; // la liste par défaut masque les reports en cours
    expect(Object.keys(scenario.summaries).length).toBe(scenario.threads.length);
    for (const [id, want] of Object.entries(scenario.summaries)) {
      expect(all.find((t) => t.id === Number(id)), id).toMatchObject(want);
    }
  });

  // La règle de CLAUDE.md (« toute évolution des filtres ajoute un cas ») devient un contrôle : chaque filtre du moteur
  // (clé de `ThreadQuery`) figure dans au moins un cas du scénario, et le scénario ne peut pas rétrécir en silence.
  it("chaque filtre de ThreadQuery est exercé par le scénario", () => {
    const keys = [...readFileSync(join(__dirname, "../bindings/ThreadQuery.ts"), "utf8").matchAll(/\b(\w+)\??: /g)].map((m) => m[1]);
    expect(keys.length).toBeGreaterThanOrEqual(10);
    for (const k of keys) expect(scenario.queries.some((q) => k in q.query), `aucun cas n'utilise « ${k} »`).toBe(true);
    expect(scenario.queries.length).toBeGreaterThanOrEqual(45);
  });
});

// Les mutations (déplacer, supprimer) passent par IMAP côté moteur : elles ne se rejouent pas dans le scénario partagé.
// La démo garde au moins la cohérence : une conversation déplacée est dans son nouveau dossier, et plus dans l'ancien.
describe("démo : déplacer une conversation", () => {
  it("elle change de dossier et de rôle", async () => {
    setDemoThreads(demoThreads());
    expect(listThreads({ role: "archive", accountId: "pro" }).map((t) => t.id)).not.toContain(1);
    await demoBackend.moveThread(1, 1, "archive");
    expect(listThreads({ role: "archive", accountId: "pro" }).map((t) => t.id)).toContain(1);
    expect(listThreads({ folderId: 1 }).map((t) => t.id)).not.toContain(1);
  });
});

