import { Archive, FileText, Inbox, Mail, Paperclip, Send, ShieldAlert, Star, Trash2 } from "lucide-react";
import type { ComponentType } from "react";

export type Role = "inbox" | "sent" | "drafts" | "archive" | "junk" | "trash";

/** Dossiers où l'on peut déplacer une conversation (miroir de `Destination` côté Rust). */
export type Destination = Extract<Role, "inbox" | "archive" | "junk" | "trash">;

export type View =
  | { kind: "role"; role: Role }
  | { kind: "starred" }
  | { kind: "unread" }
  | { kind: "attachments" }
  | { kind: "folder"; folderId: number; name: string };

export interface NavItem {
  view: View;
  label: string;
  icon: ComponentType<{ size?: number; strokeWidth?: number; className?: string }>;
  /** Raccourci « g puis lettre ». */
  go?: string;
}

const role = (r: Role): View => ({ kind: "role", role: r });

/** Source unique des vues : barre latérale, palette, titres, raccourcis G+lettre et aide en dérivent. */
export const NAV_MAILBOXES: NavItem[] = [
  { view: role("inbox"), label: "Boîte de réception", icon: Inbox, go: "i" },
  { view: { kind: "starred" }, label: "Suivis", icon: Star, go: "s" },
  { view: role("drafts"), label: "Brouillons", icon: FileText, go: "d" },
  { view: role("sent"), label: "Envoyés", icon: Send, go: "t" },
  { view: role("archive"), label: "Archives", icon: Archive, go: "a" },
  { view: role("junk"), label: "Spam", icon: ShieldAlert },
  { view: role("trash"), label: "Corbeille", icon: Trash2 },
];

export const NAV_SMART: NavItem[] = [
  { view: { kind: "unread" }, label: "Non lus", icon: Mail, go: "u" },
  { view: { kind: "attachments" }, label: "Avec pièces jointes", icon: Paperclip },
];

const ALL = [...NAV_MAILBOXES, ...NAV_SMART];

export function sameView(a: View, b: View): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "role") return a.role === (b as typeof a).role;
  if (a.kind === "folder") return a.folderId === (b as typeof a).folderId;
  return true;
}

export const isInboxView = (v: View) => v.kind === "role" && v.role === "inbox";

export const viewTitle = (v: View): string => (v.kind === "folder" ? v.name : ALL.find((i) => sameView(i.view, v))?.label ?? "");

export const GO_VIEWS: Record<string, View> = Object.fromEntries(ALL.filter((i) => i.go).map((i) => [i.go as string, i.view]));

/** Source unique des déplacements : boutons, palette, raccourcis, aide et messages en dérivent. */
export const DESTINATIONS: Record<Destination, { label: string; icon: NavItem["icon"]; key: string; done: string; doneMany: string; already: string; failed: string }> = {
  inbox: { label: "Remettre dans la boîte de réception", icon: Inbox, key: "I", done: "Conversation remise dans la boîte de réception", doneMany: "remises dans la boîte de réception", already: "Cette conversation est déjà dans la boîte de réception.", failed: "Déplacement impossible" },
  archive: { label: "Archiver", icon: Archive, key: "E", done: "Conversation archivée", doneMany: "archivées", already: "Cette conversation est déjà archivée.", failed: "Archivage impossible" },
  trash: { label: "Mettre à la corbeille", icon: Trash2, key: "#", done: "Conversation mise à la corbeille", doneMany: "mises à la corbeille", already: "Cette conversation est déjà à la corbeille.", failed: "Suppression impossible" },
  junk: { label: "Signaler comme spam", icon: ShieldAlert, key: "!", done: "Conversation signalée comme spam", doneMany: "signalées comme spam", already: "Cette conversation est déjà dans le spam.", failed: "Signalement impossible" },
};
export const isDestination = (role: string): role is Destination => Object.hasOwn(DESTINATIONS, role);
export const DESTINATION_IDS = Object.keys(DESTINATIONS) as Destination[];

/** Destinations proposées depuis cette vue : jamais la vue courante ; « remettre dans la boîte de réception » seulement depuis archives, spam et corbeille. */
export function destinationsFor(view: View): Destination[] {
  const here = view.kind === "role" ? view.role : null;
  const restorable = here === "archive" || here === "junk" || here === "trash";
  return DESTINATION_IDS.filter((d) => d !== here && (d !== "inbox" || restorable));
}

/** Type MIME du glisser-déposer d'une conversation vers un dossier de la barre latérale. */
export const DRAG_TYPE = "application/x-envelope-thread";

/** Corbeille et spam : les seules boîtes où l'on supprime pour de bon (et que l'on peut vider). */
export const isPurgeView = (v: View): v is { kind: "role"; role: "trash" | "junk" } => v.kind === "role" && (v.role === "trash" || v.role === "junk");

/** Identifiants de conversations d'un glisser (tableau JSON) ; tout autre contenu est ignoré. */
export function parseDragIds(raw: string): number[] {
  try {
    const v: unknown = JSON.parse(raw);
    return Array.isArray(v) && v.every((n) => Number.isInteger(n)) ? v : [];
  } catch {
    return [];
  }
}
