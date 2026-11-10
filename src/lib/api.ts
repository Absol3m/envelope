import { invoke } from "@tauri-apps/api/core";

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
}
export interface Addr {
  name: string | null;
  address: string;
}
export interface DetailMessage {
  id: number;
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
  archive(threadId: number, folderId: number): Promise<void>;
  syncNow(accountId: string): Promise<void>;
  addAccount(input: NewAccount): Promise<Account>;
  addOAuthAccount(provider: OAuthProviderId, email: string): Promise<Account>;
  detectServers(email: string): Promise<Detected>;
  oauthStatus(): Promise<OAuthStatus>;
  setOAuthClient(provider: OAuthProviderId, clientId: string, clientSecret?: string): Promise<void>;
  clearOAuthClient(provider: OAuthProviderId): Promise<void>;
  removeAccount(accountId: string): Promise<void>;
  openAttachment(id: number): Promise<void>;
  inlineImages(messageId: number): Promise<Record<string, string>>;
  mcpInfo(): Promise<McpInfo>;
}

export const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const tauriBackend: Backend = {
  demo: false,
  listAccounts: () => invoke("list_accounts"),
  listFolders: () => invoke("list_folders", {}),
  listThreads: (query) => invoke("list_threads", { query }),
  getThread: (threadId) => invoke("get_thread_detail", { threadId }),
  setRead: (threadId, read) => invoke("set_thread_read", { threadId, read }),
  setStarred: (threadId, starred) => invoke("set_thread_starred", { threadId, starred }),
  archive: (threadId, folderId) => invoke("archive_thread", { threadId, folderId }),
  syncNow: (accountId) => invoke("sync_now", { accountId }),
  addAccount: (input) => invoke("add_account", { input }),
  addOAuthAccount: (provider, email) => invoke("add_oauth_account", { provider, email }),
  detectServers: (email) => invoke("detect_servers", { email }),
  oauthStatus: () => invoke("oauth_status"),
  setOAuthClient: (provider, clientId, clientSecret) => invoke("set_oauth_client", { provider, clientId, clientSecret }),
  clearOAuthClient: (provider) => invoke("clear_oauth_client", { provider }),
  removeAccount: (accountId) => invoke("remove_account", { accountId }),
  openAttachment: (attachmentId) => invoke("open_attachment", { attachmentId }),
  inlineImages: (messageId) => invoke("inline_images", { messageId }),
  mcpInfo: () => invoke("mcp_info"),
};
