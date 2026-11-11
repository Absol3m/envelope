import { describe, expect, it } from "vitest";
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
  it("une conversation locale est placée dans son dossier local", () => {
    expect(placeOf({ accountId: LOCAL_ACCOUNT, folderId: 9, folderName: "Clients", folderRole: null })).toEqual(localFolder);
    expect(placeOf({ accountId: "a", folderId: 1, folderName: "INBOX", folderRole: "inbox" })).toEqual(inbox);
    expect(placeOf({ accountId: "a", folderId: 4, folderName: "Pro", folderRole: null })).toEqual({ kind: "folder", folderId: 4, name: "Pro" });
  });
});
