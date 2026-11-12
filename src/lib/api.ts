import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { ServerDestination } from "./views";

/** Types miroirs des structures Rust (src-tauri/src/commands.rs, crates/envelope-core). */
/** Adresse d'envoi supplémentaire (alias) d'un compte ; l'authentification reste celle du compte. */
export interface Identity {
  id: number;
  name: string;
  email: string;
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
  /** Texte brut, inséré dans les nouveaux messages et les réponses. */
  signature: string;
  identities: Identity[];
}
/** Dossier trouvé dans une source d'import (profil Thunderbird, dossier de boîtes mbox ou Maildir). */
export interface ImportFolder {
  key: string;
  /** Nom du dossier local créé (hiérarchie jointe par « › »). */
  name: string;
  kind: "mbox" | "maildir";
  /** Octets (mbox) ou nombre de fichiers (Maildir). */
  size: number;
  selected: boolean;
}
/** Source d'import choisie par l'utilisateur : jeton opaque, chemin à afficher, dossiers trouvés. */
export interface ImportSource {
  token: string;
  path: string;
  folders: ImportFolder[];
}
export interface ImportReport {
  imported: number;
  duplicates: number;
  /** Messages supprimés dans la source, non repris. */
  deleted: number;
  failed: number;
}
export interface ImportProgress {
  folder: string;
  imported: number;
  foldersDone: number;
  foldersTotal: number;
}
export interface ExportReport {
  exported: number;
  /** Messages dont le message brut est introuvable (non exportés). */
  missing: number;
}
export type ExportFormat = "mbox" | "eml";
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
  /** Rappel local : report ou relance si pas de réponse ; échéance et, une fois échu, heure de retour (secondes Unix). */
  reminderKind: ReminderKind | null;
  reminderDue: number | null;
  reminderFired: number | null;
}
export type ReminderKind = "snooze" | "followup";
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
  /** Vérification d'identité de l'expéditeur échouée d'après le serveur de réception (« dmarc », « spf », « dkim ») : possible usurpation. */
  authFail: "dmarc" | "spf" | "dkim" | null;
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
  /** Conversations avec un rappel en attente. */
  remindersOnly?: boolean;
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
  /** Brouillon dont ce message est la version finale : supprimé du serveur après l'envoi. */
  draftId?: string;
  /** Adresse d'expéditeur : celle du compte (défaut) ou l'un de ses alias. */
  fromEmail?: string;
}
/** Brouillon relu depuis le serveur, prêt à rouvrir dans le composer. */
export interface DraftData {
  id: string;
  accountId: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  bodyHtml: string | null;
  bodyText: string;
  inReplyTo: string | null;
  references: string[];
  attachments: { filename: string; mimeType: string; dataBase64: string }[];
  fromEmail: string;
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

/** Envoi en file : parti à `sendAt` si l'application est ouverte. */
export interface ScheduledSend {
  id: number;
  accountId: string;
  /** Secondes Unix. */
  sendAt: number;
  subject: string;
  to: string[];
  status: "pending" | "sending" | "failed";
  error: string | null;
}

export type LockMethod = "off" | "system" | "password";
/** État du verrouillage de l'application (voir `src-tauri/src/lock.rs`). */
export interface LockStatus {
  method: LockMethod;
  locked: boolean;
  /** Verrouillage automatique après cette durée d'inactivité, en secondes (0 = jamais). */
  timeoutSecs: number;
  /** Le système offre une authentification native (Touch ID, Windows Hello…) ici. */
  systemAvailable: boolean;
}

export type DiskEncryption = "on" | "off" | "unknown";

export type RuleMove = "archive" | "trash" | "junk";
/** Règle locale : condition en syntaxe de recherche (`de:`, `objet:`, `pj`…), appliquée aux nouveaux messages reçus. */
export interface Rule {
  /** 0 à la création. */
  id: number;
  name: string;
  enabled: boolean;
  /** `null` : tous les comptes. */
  accountId: string | null;
  query: string;
  markRead: boolean;
  star: boolean;
  moveTo: RuleMove | null;
}
export interface Template {
  id: number;
  name: string;
  subject: string;
  bodyHtml: string;
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
  /** Sélecteur de dossier natif, ouvert par l'application, puis liste des dossiers importables. `null` si annulé. */
  importPick(): Promise<ImportSource | null>;
  /** `token` : celui d'`importPick` (l'interface ne fournit jamais de chemin). */
  importRun(token: string, keys: string[]): Promise<ImportReport>;
  /** Sélecteur natif de la destination (fichier pour mbox, dossier pour eml) ; renvoie un jeton, `null` si annulé. */
  exportPick(format: ExportFormat, defaultName: string): Promise<string | null>;
  /** `token` : celui d'`exportPick`. */
  exportFolder(folderId: number, format: ExportFormat, token: string): Promise<ExportReport>;
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
  /** Renouvelle l'autorisation d'un compte OAuth2 (jeton expiré ou révoqué). */
  reconnectOAuthAccount(accountId: string): Promise<void>;
  detectServers(email: string): Promise<Detected>;
  oauthStatus(): Promise<OAuthStatus>;
  setOAuthClient(provider: OAuthProviderId, clientId: string, clientSecret?: string): Promise<void>;
  clearOAuthClient(provider: OAuthProviderId): Promise<void>;
  sendEmail(accountId: string, message: OutgoingMessage): Promise<SendReport>;
  /** Enregistre le brouillon sur le serveur (nouvelle version, puis suppression des précédentes). */
  saveDraft(accountId: string, message: OutgoingMessage): Promise<void>;
  deleteDraft(accountId: string, draftId: string): Promise<void>;
  /** Relit le brouillon d'une conversation (dossier Brouillons) pour le rouvrir. */
  openDraft(threadId: number): Promise<DraftData>;
  suggestContacts(accountId: string | null, query: string): Promise<Addr[]>;
  removeAccount(accountId: string): Promise<void>;
  setAccountSignature(accountId: string, signature: string): Promise<void>;
  /** Met le message en file pour `sendAt` (secondes Unix) ; renvoie l'identifiant de l'envoi programmé. */
  scheduleSend(accountId: string, message: OutgoingMessage, sendAt: number): Promise<number>;
  /** Pose ou remplace le rappel de la conversation ; `dueAt` en secondes Unix. */
  setReminder(threadId: number, kind: ReminderKind, dueAt: number): Promise<void>;
  clearReminder(threadId: number): Promise<void>;
  listScheduled(): Promise<ScheduledSend[]>;
  /** Retire l'envoi de la file et rend le message, pour le rouvrir. */
  cancelScheduled(id: number): Promise<OutgoingMessage>;
  sendScheduledNow(id: number): Promise<void>;
  addIdentity(accountId: string, name: string, email: string): Promise<void>;
  removeIdentity(id: number): Promise<void>;
  listRules(): Promise<Rule[]>;
  /** Crée (`id` 0) ou remplace la règle ; rend son identifiant. */
  saveRule(rule: Rule): Promise<number>;
  deleteRule(id: number): Promise<void>;
  /** Applique les règles aux conversations récentes de la boîte de réception ayant du courrier non lu ; rend le nombre de conversations traitées. */
  runRulesNow(): Promise<number>;
  listTemplates(): Promise<Template[]>;
  /** Un modèle du même nom est remplacé. */
  saveTemplate(template: Omit<Template, "id">): Promise<Template>;
  deleteTemplate(id: number): Promise<void>;
  /** Désabonne de la liste d'un message (POST en un clic ou message mailto). */
  unsubscribe(messageId: number): Promise<void>;
  openAttachment(id: number): Promise<void>;
  inlineImages(messageId: number): Promise<Record<string, string>>;
  lockStatus(): Promise<LockStatus>;
  unlockPassword(password: string): Promise<void>;
  /** Ouvre l'invite d'authentification du système (Touch ID, Windows Hello) ; se termine quand elle est close. */
  unlockSystem(): Promise<void>;
  lockNow(): Promise<void>;
  /** Secours verrouillé : efface les données locales et le verrouillage (les e-mails restent sur les serveurs). */
  resetApp(): Promise<void>;
  /** `currentPassword` : exigé pour modifier un verrouillage par mot de passe déjà actif. */
  setLock(method: LockMethod, timeoutSecs: number, password?: string, currentPassword?: string): Promise<LockStatus>;
  /** Le disque qui contient les données est-il chiffré par le système (FileVault, BitLocker, LUKS) ? */
  diskEncryption(): Promise<DiskEncryption>;
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
  importPick: () => invoke("import_pick"),
  importRun: (token, keys) => invoke("import_run", { token, keys }),
  exportPick: (format, defaultName) => invoke("export_pick", { format, defaultName }),
  exportFolder: (folderId, format, token) => invoke("export_folder", { folderId, format, token }),
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
  reconnectOAuthAccount: (id) => invoke("reconnect_oauth_account", { id }),
  detectServers: (email) => invoke("detect_servers", { email }),
  oauthStatus: () => invoke("oauth_status"),
  setOAuthClient: (provider, clientId, clientSecret) => invoke("set_oauth_client", { provider, clientId, clientSecret }),
  clearOAuthClient: (provider) => invoke("clear_oauth_client", { provider }),
  sendEmail: (accountId, message) => invoke("send_email", { accountId, message }),
  saveDraft: (accountId, message) => invoke("save_draft", { accountId, message }),
  deleteDraft: (accountId, draftId) => invoke("delete_draft", { accountId, draftId }),
  openDraft: (threadId) => invoke("open_draft", { threadId }),
  suggestContacts: (accountId, query) => invoke("suggest_contacts", { accountId, query }),
  removeAccount: (accountId) => invoke("remove_account", { accountId }),
  unsubscribe: (messageId) => invoke("unsubscribe", { messageId }),
  setAccountSignature: (accountId, signature) => invoke("set_account_signature", { accountId, signature }),
  scheduleSend: (accountId, message, sendAt) => invoke("schedule_send", { accountId, message, sendAt }),
  setReminder: (threadId, kind, dueAt) => invoke("set_reminder", { threadId, kind, dueAt }),
  clearReminder: (threadId) => invoke("clear_reminder", { threadId }),
  listScheduled: () => invoke("list_scheduled"),
  cancelScheduled: (id) => invoke("cancel_scheduled", { id }),
  sendScheduledNow: (id) => invoke("send_scheduled_now", { id }),
  addIdentity: (accountId, name, email) => invoke("add_identity", { accountId, name, email }),
  removeIdentity: (id) => invoke("remove_identity", { id }),
  listRules: () => invoke("list_rules"),
  saveRule: (rule) => invoke("save_rule", { rule }),
  deleteRule: (id) => invoke("delete_rule", { id }),
  runRulesNow: () => invoke("run_rules_now"),
  listTemplates: () => invoke("list_templates"),
  saveTemplate: (template) => invoke("save_template", { template }),
  deleteTemplate: (id) => invoke("delete_template", { id }),
  openAttachment: (attachmentId) => invoke("open_attachment", { attachmentId }),
  inlineImages: (messageId) => invoke("inline_images", { messageId }),
  lockStatus: () => invoke("lock_status"),
  unlockPassword: (password) => invoke("unlock_password", { password }),
  unlockSystem: () => invoke("unlock_system"),
  lockNow: () => invoke("lock_now"),
  resetApp: () => invoke("reset_app"),
  setLock: (method, timeoutSecs, password, currentPassword) => invoke("set_lock", { method, timeoutSecs, password, currentPassword }),
  diskEncryption: () => invoke("disk_encryption"),
  mcpInfo: () => invoke("mcp_info"),
};
