import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { DetailMessage, ThreadQuery } from "../api";
import { listThreads, setDemoThreads, type MockThread } from "../mock";

/** Contrat de la liste : même scénario que le moteur Rust (src-tauri/src/mail/list_contract.rs), mêmes réponses attendues. */
interface Scenario {
  folders: { id: number; account: string; role: MockThread["role"] }[];
  threads: { id: number; account: string; folder: number; starred: boolean; attachments: boolean; reminder?: MockThread["reminder"]; messages: { from: [string, string]; subject: string; date: string; read: boolean; text: string }[] }[];
  queries: { name: string; query: ThreadQuery; ids: number[] }[];
  summaries: Record<string, Record<string, unknown>>;
}
const scenario = JSON.parse(readFileSync(join(__dirname, "../../../src-tauri/tests/contract/list_threads.json"), "utf8")) as Scenario;

let nextId = 1;
const demoThreads = (): MockThread[] =>
  scenario.threads.map((t) => {
    const folderId = t.folder;
    const messages: DetailMessage[] = t.messages.map((m) => ({
      id: nextId++, messageId: null, references: [], folderId, subject: m.subject,
      fromName: m.from[0], fromAddr: m.from[1], to: [], cc: [], date: m.date, bodyText: m.text, bodyHtml: null,
      isRead: m.read, isStarred: false, unsubscribe: null, authFail: null,
    }));
    return {
      id: t.id, accountId: t.account, role: scenario.folders.find((f) => f.id === folderId)!.role, starred: t.starred, messages,
      attachments: t.attachments ? [{ id: t.id, messageId: messages[0].id, filename: "pj.pdf", mimeType: "application/pdf", size: 1, contentId: null, isInline: false, previewable: false }] : [],
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

  it("dossier de chaque conversation : mêmes identifiants que le scénario", () => {
    for (const t of listThreads({ limit: 1000 })) {
      expect(t.folderId, `conversation ${t.id}`).toBe(scenario.threads.find((x) => x.id === t.id)!.folder);
    }
  });

  it("résumés : compteurs, dernier expéditeur, participants, dossier", () => {
    const all = listThreads({});
    for (const [id, want] of Object.entries(scenario.summaries)) {
      expect(all.find((t) => t.id === Number(id)), id).toMatchObject(want);
    }
  });
});
