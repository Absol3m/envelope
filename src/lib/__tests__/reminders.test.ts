// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, ThreadSummary } from "../api";
import { followupOptions } from "../compose";
import { demoBackend } from "../mock";
import { useMail } from "../store";

let setReminder: ReturnType<typeof vi.fn>;
let clearReminder: ReturnType<typeof vi.fn>;
const thread = (id: number, over: Partial<ThreadSummary> = {}): ThreadSummary => ({ id, accountId: "a", subject: `T${id}`, snippet: "", lastMessageAt: "2026-01-01T00:00:00Z", messageCount: 1, unreadCount: 0, isStarred: false, hasAttachments: false, fromName: null, fromAddr: null, participants: null, folderId: 1, reminderKind: null, reminderDue: null, reminderFired: null, ...over });

beforeEach(() => {
  setReminder = vi.fn(() => Promise.resolve());
  clearReminder = vi.fn(() => Promise.resolve());
  useMail.setState({
    backend: { ...demoBackend, demo: false, setReminder, clearReminder, listFolders: () => Promise.resolve([]) } as Backend,
    threads: [thread(1), thread(2), thread(3, { reminderKind: "followup", reminderDue: 5, reminderFired: 9 })],
    selectedId: null, toasts: [], view: { kind: "role", role: "inbox" },
  });
});

describe("rappels", () => {
  it("un report sort la conversation de la liste et envoie l'échéance en secondes", () => {
    useMail.getState().remind(1, "snooze", 1_700_000_000_500);
    expect(useMail.getState().threads.map((t) => t.id)).toEqual([2, 3]);
    expect(setReminder).toHaveBeenCalledWith(1, "snooze", 1_700_000_000);
  });

  it("une relance garde la conversation visible, avec sa pastille", () => {
    useMail.getState().remind(2, "followup", 2_000_000_000_000);
    expect(useMail.getState().threads.find((t) => t.id === 2)).toMatchObject({ reminderKind: "followup", reminderDue: 2_000_000_000, reminderFired: null });
  });

  it("dans la vue « Rappels », un report reste affiché et annuler le retire", () => {
    useMail.setState({ view: { kind: "reminders" } });
    useMail.getState().remind(1, "snooze", 2_000_000_000_000);
    expect(useMail.getState().threads.map((t) => t.id)).toEqual([1, 2, 3]);
    useMail.getState().clearReminder(1);
    expect(useMail.getState().threads.map((t) => t.id)).toEqual([2, 3]);
    expect(clearReminder).toHaveBeenCalledWith(1);
  });

  it("ouvrir une conversation revenue acquitte son rappel", () => {
    useMail.getState().select(3);
    expect(clearReminder).toHaveBeenCalledWith(3);
    expect(useMail.getState().threads.find((t) => t.id === 3)?.reminderKind).toBeNull();
  });

  it("délais de relance : 3 jours, 1 semaine, 2 semaines, à 8 h", () => {
    const o = followupOptions(new Date("2026-11-11T14:00:00"));
    expect(o.map((x) => [x.label, x.at.getDate(), x.at.getHours()])).toEqual([["Dans 3 jours", 14, 8], ["Dans 1 semaine", 18, 8], ["Dans 14 jours", 25, 8]]);
  });
});
