import { describe, expect, it } from "vitest";
import type { FolderRow } from "../api";
import { destinationsFor, isEmptiableView, isPurgeView, LOCAL_ACCOUNT, placeOf, type View } from "../views";

const inbox: View = { kind: "role", role: "inbox" };
const localFolder: View = { kind: "folder", folderId: 9, name: "Clients", local: true };

describe("dossiers locaux", () => {
  it("l'archivage local est proposé depuis les boîtes d'un compte, pas depuis un dossier local", () => {
    expect(destinationsFor(inbox)).toContain("local");
    expect(destinationsFor({ kind: "role", role: "trash" })).toContain("local");
    expect(destinationsFor(localFolder)).toEqual([]);
  });
  it("on supprime pour de bon depuis la corbeille, le spam et les dossiers locaux ; on ne vide que corbeille et spam", () => {
    expect(isPurgeView(localFolder)).toBe(true);
    expect(isPurgeView({ kind: "folder", folderId: 3, name: "Pro" })).toBe(false);
    expect(isPurgeView({ kind: "role", role: "junk" })).toBe(true);
    expect(isEmptiableView(localFolder)).toBe(false);
    expect(isEmptiableView({ kind: "role", role: "trash" })).toBe(true);
  });
  it("une conversation est placée dans le dossier de son dernier message, d'après la liste des dossiers", () => {
    const f = (id: number, accountId: string, name: string, role: FolderRow["role"]): FolderRow => ({ id, accountId, path: name, name, role, unreadCount: 0, totalCount: 0 });
    const folders = [f(1, "a", "INBOX", "inbox"), f(4, "a", "Pro", null), f(9, LOCAL_ACCOUNT, "Clients", null)];
    expect(placeOf({ folderId: 9 }, folders)).toEqual(localFolder);
    expect(placeOf({ folderId: 1 }, folders)).toEqual(inbox);
    expect(placeOf({ folderId: 4 }, folders)).toEqual({ kind: "folder", folderId: 4, name: "Pro" });
    expect(placeOf({ folderId: 77 }, folders)).toBeNull(); // dossier inconnu
    expect(placeOf({ folderId: null }, folders)).toBeNull();
  });
});
