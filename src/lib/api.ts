import { invoke } from "@tauri-apps/api/core";

/** Types miroirs des structures Rust (src-tauri/src/commands.rs, src-tauri/src/mail/*). */
export interface AppInfo {
  name: string;
  version: string;
  dbPath: string;
}
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
export interface MessageSummary {
  id: number;
  accountId: string;
  folderId: number;
  threadId: number | null;
  subject: string;
  fromName: string | null;
  fromAddr: string;
  date: string;
  snippet: string;
  isRead: boolean;
  isStarred: boolean;
  hasAttachments: boolean;
}
export interface NewAccount {
  email: string;
  displayName?: string;
  username?: string;
  password: string;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (input: NewAccount) => invoke<Account>("add_account", { input }),
  addOAuthAccount: (provider: "google" | "microsoft", email: string) =>
    invoke<Account>("add_oauth_account", { provider, email }),
  removeAccount: (accountId: string) => invoke<void>("remove_account", { accountId }),
  syncNow: (accountId: string) => invoke<void>("sync_now", { accountId }),
  listFolders: (accountId?: string) => invoke<FolderRow[]>("list_folders", { accountId }),
  listMessages: (folderId: number, limit = 50, offset = 0) =>
    invoke<MessageSummary[]>("list_messages", { folderId, limit, offset }),
  searchEmails: (query: string, accountId?: string, limit = 50) =>
    invoke<MessageSummary[]>("search_emails", { query, accountId, limit }),
};
