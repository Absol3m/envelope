import { describe, expect, it } from "vitest";
import { demoBackend } from "../mock";

describe("message volumineux (démo)", () => {
  it("n'a que l'en-tête jusqu'au téléchargement", async () => {
    const threads = await demoBackend.listThreads({});
    let target: { id: number; messageId: number } | undefined;
    for (const t of threads) {
      const d = await demoBackend.getThread(t.id);
      const m = d.messages.find((x) => x.oversize);
      if (m) target = { id: t.id, messageId: m.id };
    }
    expect(target).toBeDefined();
    const before = (await demoBackend.getThread(target!.id)).messages.find((m) => m.id === target!.messageId)!;
    expect(before.oversize && before.size).toBeGreaterThan(50 * 1024 * 1024);
    expect(before.bodyText).toBeNull();
    await demoBackend.downloadMessage(target!.messageId);
    const after = (await demoBackend.getThread(target!.id)).messages.find((m) => m.id === target!.messageId)!;
    expect(after.oversize).toBe(false);
    expect(after.bodyText).toBeTruthy();
  });
});
