import { Archive, FileText, Inbox, Mail, Paperclip, Send, ShieldAlert, Star, Trash2 } from "lucide-react";
import type { ComponentType } from "react";

export type Role = "inbox" | "sent" | "drafts" | "archive" | "junk" | "trash";

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
