import { invoke } from "@tauri-apps/api/core";

/** Types miroirs des structures Rust (src-tauri/src/commands.rs). */
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
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
};
