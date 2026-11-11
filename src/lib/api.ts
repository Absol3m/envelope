import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { ServerDestination } from "./views";

/** Types miroirs des structures Rust (src-tauri/src/commands.rs, crates/envelope-core). */
export interface Account {
  id: string;
  email: string;
  displayName: string | null;
  provider: string;
  authKind: "password" | "oauth2";
  syncStatus: "idle" | "syncing" | "error";
  syncError: string | null;
  lastSyncAt: string | null;
  /** Texte brut, inséré dans les nouveaux messages et les réponses. */
  signature: string;
}
export interface FolderRow {
  id: number;
  accountId: string;
  path: string;
  name: string;
  role: string | null;
  unreadCount: number;
  totalCount: number;
}
export interface ThreadSummary {
  id: number;
  accountId: string;
  subject: string;
  snippet: string;
  lastMessageAt: string;
  messageCount: number;
  unreadCount: number;
  isStarred: boolean;
  hasAttachments: boolean;
  fromName: string | null;
  fromAddr: string | null;
  participants: string | null;
  /** Dossier du dernier message : les résultats de recherche couvrent tous les dossiers. */
  folderId: number | null;
  folderName: string | null;
  folderRole: string | null;
}
export interface Addr {
  name: string | null;
  address: string;
}
export interface DetailMessage {
  id: number;
  messageId: string | null;
  references: string[];
  folderId: number;
  subject: string;
  fromName: string | null;
  fromAddr: string;
  to: Addr[];
  cc: Addr[];
  date: string;
  bodyText: string | null;
  bodyHtml: string | null;
  isRead: boolean;
  isStarred: boolean;
  /** Désabonnement proposé par l'expéditeur (en-têtes List-Unsubscribe). */
  unsubscribe: Unsubscribe | null;
}
export interface Unsubscribe {
  url: string | null;
  mailto: string | null;
  /** Un simple POST suffit (RFC 8058) : désabonnement sans page intermédiaire. */
  oneClick: boolean;
}
export interface DetailAttachment {
  id: number;
  messageId: number;
  filename: string | null;
  mimeType: string;
  size: number;
  contentId: string | null;
  isInline: boolean;
}
export interface ThreadDetail {
  id: number;
  accountId: string;
  subject: string;
  isStarred: boolean;
  messages: DetailMessage[];
  attachments: DetailAttachment[];
}
export interface ThreadQuery {
  folderId?: number;
  role?: string;
  accountId?: string;
  starredOnly?: boolean;
  unreadOnly?: boolean;
  withAttachments?: boolean;
  query?: string;
  limit?: number;
  offset?: number;
}
export interface NewAccount {
  email: string;
  displayName?: string;
  password: string;
  imap?: ServerConfig;
  smtp?: ServerConfig;
}
export interface ServerConfig {
  host: string;
  port: number;
  security: "tls" | "starttls" | "none";
}
/** Serveurs détectés pour une adresse (`source` : builtin | autoconfig | ispdb | mx | guess). */
export interface Detected {
  provider: string;
  imap: ServerConfig;
  smtp: ServerConfig;
  username: string;
  oauth: OAuthProviderId | null;
  source: "builtin" | "autoconfig" | "ispdb" | "mx" | "guess";
}
export type OAuthProviderId = "google" | "microsoft";
export type OAuthStatus = Record<OAuthProviderId, boolean>;
/** Message à envoyer (miroir de `OutgoingMessage` côté Rust). Adresses au format « Nom <adresse> » ou « adresse ». */
export interface OutgoingMessage {
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  bodyText: string;
  bodyHtml?: string;
  inReplyTo?: string;
  references: string[];
  attachments: { filename: string; mimeType: string; dataBase64: string }[];
}
/** Sort de la copie dans « Envoyés » : déposée, rangée par le serveur (Gmail) ou impossible (cause). */
export type SentCopy = "saved" | "server" | { failed: string };
export interface SendReport {
  sentCopy: SentCopy;
}
export interface McpInfo {
  binaryPath: string | null;
  dbPath: string;
}

/** Tout ce dont l'interface a besoin du « serveur » : l'implémentation réelle (Tauri) ou la démo. */
export interface Backend {
  readonly demo: boolean;
  listAccounts(): Promise<Account[]>;
  listFolders(): Promise<FolderRow[]>;
  listThreads(q: ThreadQuery): Promise<ThreadSummary[]>;
  getThread(id: number): Promise<ThreadDetail>;
  setRead(threadId: number, read: boolean): Promise<void>;
  setStarred(threadId: number, starred: boolean): Promise<void>;
  moveThread(threadId: number, folderId: number, to: ServerDestination): Promise<void>;
  deleteThread(threadId: number, folderId: number): Promise<void>;
  /** Archive en local (copie vérifiée sur cet ordinateur, puis suppression sur le serveur) ; entre dossiers locaux : simple déplacement. */
  archiveLocal(threadId: number, folderId: number, localFolderId: number): Promise<void>;
  createLocalFolder(name: string): Promise<number>;
  renameLocalFolder(folderId: number, name: string): Promise<void>;
  /** Seulement si le dossier est vide. */
  deleteLocalFolder(folderId: number): Promise<void>;
  emptyFolder(folderId: number): Promise<void>;
  /** Confirmation native (boîte de dialogue de l'OS) avant une action irréversible. */
  confirm(message: string, okLabel: string): Promise<boolean>;
  syncNow(accountId: string): Promise<void>;
  addAccount(input: NewAccount): Promise<Account>;
  addOAuthAccount(provider: OAuthProviderId, email: string): Promise<Account>;
  detectServers(email: string): Promise<Detected>;
  oauthStatus(): Promise<OAuthStatus>;
  setOAuthClient(provider: OAuthProviderId, clientId: string, clientSecret?: string): Promise<void>;
  clearOAuthClient(provider: OAuthProviderId): Promise<void>;
  sendEmail(accountId: string, message: OutgoingMessage): Promise<SendReport>;
  suggestContacts(accountId: string | null, query: string): Promise<Addr[]>;
  removeAccount(accountId: string): Promise<void>;
  setAccountSignature(accountId: string, signature: string): Promise<void>;
  /** Désabonne de la liste d'un message (POST en un clic ou message mailto). */
  unsubscribe(messageId: number): Promise<void>;
  openAttachment(id: number): Promise<void>;
  inlineImages(messageId: number): Promise<Record<string, string>>;
  mcpInfo(): Promise<McpInfo>;
  /** Pastille de non-lus sur l'icône de l'application (0 : aucune). */
  setBadge(count: number): Promise<void>;
}

export const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Réglages d'apparence du système hôte (crate envelope-appearance, `read`). */
export interface SystemAppearance {
  /** Préférence sombre du bureau, quand la vue web ne la voit pas (Linux). */
  dark: boolean | null;
  accentLight: string | null;
  accentDark: string | null;
  /** Schéma de couleurs de l'utilisateur (KDE). */
  palette: { window: string; view: string; fg: string; fgMuted: string } | null;
}

export const systemAppearance = () => invoke<SystemAppearance>("system_appearance");

export const tauriBackend: Backend = {
  demo: false,
  listAccounts: () => invoke("list_accounts"),
  listFolders: () => invoke("list_folders", {}),
  listThreads: (query) => invoke("list_threads", { query }),
  getThread: (threadId) => invoke("get_thread_detail", { threadId }),
  setRead: (threadId, read) => invoke("set_thread_read", { threadId, read }),
  setStarred: (threadId, starred) => invoke("set_thread_starred", { threadId, starred }),
  moveThread: (threadId, folderId, to) => invoke("move_thread", { threadId, folderId, to }),
  deleteThread: (threadId, folderId) => invoke("delete_thread", { threadId, folderId }),
  archiveLocal: (threadId, folderId, localFolderId) => invoke("archive_local", { threadId, folderId, localFolderId }),
  createLocalFolder: (name) => invoke("create_local_folder", { name }),
  renameLocalFolder: (folderId, name) => invoke("rename_local_folder", { folderId, name }),
  deleteLocalFolder: (folderId) => invoke("delete_local_folder", { folderId }),
  emptyFolder: (folderId) => invoke("empty_folder", { folderId }),
  setBadge: async (count) => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setBadgeCount(count > 0 ? count : undefined);
  },
  confirm: (message, okLabel) => ask(message, { title: "Envelope", kind: "warning", okLabel, cancelLabel: "Annuler" }),
  syncNow: (accountId) => invoke("sync_now", { accountId }),
  addAccount: (input) => invoke("add_account", { input }),
  addOAuthAccount: (provider, email) => invoke("add_oauth_account", { provider, email }),
  detectServers: (email) => invoke("detect_servers", { email }),
  oauthStatus: () => invoke("oauth_status"),
  setOAuthClient: (provider, clientId, clientSecret) => invoke("set_oauth_client", { provider, clientId, clientSecret }),
  clearOAuthClient: (provider) => invoke("clear_oauth_client", { provider }),
  sendEmail: (accountId, message) => invoke("send_email", { accountId, message }),
  suggestContacts: (accountId, query) => invoke("suggest_contacts", { accountId, query }),
  removeAccount: (accountId) => invoke("remove_account", { accountId }),
  unsubscribe: (messageId) => invoke("unsubscribe", { messageId }),
  setAccountSignature: (accountId, signature) => invoke("set_account_signature", { accountId, signature }),
  openAttachment: (attachmentId) => invoke("open_attachment", { attachmentId }),
  inlineImages: (messageId) => invoke("inline_images", { messageId }),
  mcpInfo: () => invoke("mcp_info"),
};
