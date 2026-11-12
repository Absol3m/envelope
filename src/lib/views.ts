import { AlarmClock, Archive, FileText, HardDrive, Inbox, Mail, Paperclip, Send, ShieldAlert, Star, Trash2 } from "lucide-react";
import type { ComponentType } from "react";
import type { FolderRow } from "./api";
import { t } from "./i18n";

export const ROLES = ["inbox", "sent", "drafts", "archive", "junk", "trash"] as const;
export type Role = (typeof ROLES)[number];

/** Dossiers où l'on peut déplacer une conversation (miroir de `Destination` côté Rust). */
export type Destination = Extract<Role, "inbox" | "archive" | "junk" | "trash"> | "local";

/** Destinations qui passent par le serveur (tout sauf l'archivage local). */
export type ServerDestination = Exclude<Destination, "local">;

/** Compte spécial « Sur cet ordinateur » : dossiers locaux, sans serveur (miroir de `account::LOCAL_ID`). */
export const LOCAL_ACCOUNT = "local";

export type View =
  | { kind: "role"; role: Role }
  | { kind: "starred" }
  | { kind: "unread" }
  | { kind: "attachments" }
  | { kind: "reminders" }
  | { kind: "folder"; folderId: number; name: string; /** Dossier local (sans serveur). */ local?: boolean };

export interface NavItem {
  view: View;
  label: string;
  icon: ComponentType<{ size?: number; strokeWidth?: number; className?: string }>;
  /** Raccourci « g puis lettre ». */
  go?: string;
}

const role = (r: Role): View => ({ kind: "role", role: r });

/** Entrée de navigation : le libellé est lu à l'affichage (langue active), jamais figé à l'import. */
const nav = (view: View, key: string, icon: NavItem["icon"], go?: string): NavItem => ({
  view,
  icon,
  go,
  get label() {
    return t(`nav:views.${key}`);
  },
});

/** Source unique des vues : barre latérale, palette, titres, raccourcis G+lettre et aide en dérivent. */
export const NAV_MAILBOXES: NavItem[] = [
  nav(role("inbox"), "inbox", Inbox, "i"),
  nav({ kind: "starred" }, "starred", Star, "s"),
  nav(role("drafts"), "drafts", FileText, "d"),
  nav(role("sent"), "sent", Send, "t"),
  nav(role("archive"), "archive", Archive, "a"),
  nav(role("junk"), "junk", ShieldAlert),
  nav(role("trash"), "trash", Trash2),
];

export const NAV_SMART: NavItem[] = [
  nav({ kind: "unread" }, "unread", Mail, "u"),
  nav({ kind: "attachments" }, "attachments", Paperclip),
  nav({ kind: "reminders" }, "reminders", AlarmClock),
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

/** Textes d'un déplacement, lus à l'affichage (langue active). */
interface DestinationInfo {
  icon: NavItem["icon"];
  key: string;
  readonly label: string;
  readonly done: string;
  readonly already: string;
  readonly failed: string;
  /** « 3 conversations archivées » (`count` ≥ 2). */
  doneMany(count: number): string;
}
const dest = (id: Destination, icon: NavItem["icon"], key: string): DestinationInfo => ({
  icon,
  key,
  get label() { return t(`nav:moves.${id}.label`); },
  get done() { return t(`nav:moves.${id}.done`); },
  get already() { return t(`nav:moves.${id}.already`); },
  get failed() { return t(`nav:moves.${id}.failed`); },
  doneMany: (count) => t(`nav:moves.${id}.doneMany`, { count }),
});

/** Source unique des déplacements : boutons, palette, raccourcis, aide et messages en dérivent. */
export const DESTINATIONS: Record<Destination, DestinationInfo> = {
  inbox: dest("inbox", Inbox, "I"),
  archive: dest("archive", Archive, "E"),
  trash: dest("trash", Trash2, "#"),
  local: dest("local", HardDrive, "L"),
  junk: dest("junk", ShieldAlert, "!"),
};
export const isDestination = (role: string): role is Destination => Object.hasOwn(DESTINATIONS, role);
export const DESTINATION_IDS = Object.keys(DESTINATIONS) as Destination[];

/** Destinations proposées depuis cette vue : jamais la vue courante ; « remettre dans la boîte de réception » seulement depuis archives, spam et corbeille. */
export function destinationsFor(view: View): Destination[] {
  if (view.kind === "folder" && view.local) return NO_DESTINATIONS; // les dossiers locaux n'ont pas de serveur : on y déplace par glisser
  const here = view.kind === "role" ? view.role : null;
  let list = DESTINATIONS_BY_PLACE.get(here);
  if (!list) {
    const restorable = here === "archive" || here === "junk" || here === "trash";
    list = DESTINATION_IDS.filter((d) => d !== here && (d !== "inbox" || restorable));
    DESTINATIONS_BY_PLACE.set(here, list);
  }
  return list; // même tableau d'un appel à l'autre : les lignes de la liste restent mémoïsées
}
const DESTINATIONS_BY_PLACE = new Map<Role | null, Destination[]>();
const NO_DESTINATIONS: Destination[] = [];

/** Où se trouve une conversation : le dossier de son dernier message (résultats de recherche, tous dossiers confondus). */
export function placeOf(t: { folderId: number | null }, folders: readonly FolderRow[]): View | null {
  const f = t.folderId === null ? undefined : folders.find((x) => x.id === t.folderId);
  if (!f) return null;
  if (f.accountId === LOCAL_ACCOUNT) return { kind: "folder", folderId: f.id, name: f.name, local: true };
  return f.role && (ROLES as readonly string[]).includes(f.role) ? { kind: "role", role: f.role as Role } : { kind: "folder", folderId: f.id, name: f.name };
}

/** Type MIME du glisser-déposer d'une conversation vers un dossier de la barre latérale. */
export const DRAG_TYPE = "application/x-envelope-thread";

/** Vue où l'on peut supprimer pour de bon : corbeille, spam, et dossiers locaux (où rien d'autre ne retient le message). */
export const isPurgeView = (v: View): boolean => (v.kind === "role" && (v.role === "trash" || v.role === "junk")) || (v.kind === "folder" && v.local === true);
/** Corbeille ou spam d'un compte : les seules vues qu'on peut vider d'un geste. */
export const isEmptiableView = (v: View): v is { kind: "role"; role: "trash" | "junk" } => v.kind === "role" && (v.role === "trash" || v.role === "junk");

/** Identifiants de conversations d'un glisser (tableau JSON) ; tout autre contenu est ignoré. */
export function parseDragIds(raw: string): number[] {
  try {
    const v: unknown = JSON.parse(raw);
    return Array.isArray(v) && v.every((n) => Number.isInteger(n)) ? v : [];
  } catch {
    return [];
  }
}
