import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type Account, type FolderRow, type MessageSummary } from "@/lib/api";

/** Panneau TEMPORAIRE pour tester le moteur mail (étape 2). Remplacé par l'UI de l'étape 4. */
export default function DevPanel() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [folders, setFolders] = useState<FolderRow[]>([]);
  const [messages, setMessages] = useState<MessageSummary[]>([]);
  const [form, setForm] = useState({ email: "", password: "" });
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setAccounts(await api.listAccounts());
    setFolders(await api.listFolders());
  }, []);

  useEffect(() => {
    refresh().catch((e) => setError(String(e)));
    const subs = [listen("mail:status", () => refresh()), listen("mail:synced", () => refresh())];
    return () => { subs.forEach((s) => s.then((un) => un())); };
  }, [refresh]);

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true); setError(null);
    try { await fn(); await refresh(); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };

  const input = "w-full rounded border border-border bg-background px-2 py-1";
  const btn = "rounded border border-border px-2 py-1 hover:bg-muted disabled:opacity-50";

  return (
    <div className="space-y-4 overflow-auto p-4 pt-10 text-sm">
      <h2 className="font-semibold">Comptes (panneau de test)</h2>
      {accounts.map((a) => (
        <div key={a.id} className="flex items-center gap-2">
          <span className="flex-1 truncate">{a.email} — {a.syncStatus}{a.syncError ? ` (${a.syncError})` : ""}</span>
          <button className={btn} onClick={() => api.syncNow(a.id)}>Sync</button>
          <button className={btn} onClick={() => run(() => api.removeAccount(a.id))}>×</button>
        </div>
      ))}
      <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); run(() => api.addAccount(form)); }}>
        <input className={input} placeholder="E-mail" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} />
        <input className={input} type="password" placeholder="Mot de passe (d'application)" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} />
        <div className="flex gap-2">
          <button className={btn} disabled={busy}>Ajouter</button>
          <button type="button" className={btn} disabled={busy || !form.email} onClick={() => run(() => api.addOAuthAccount("google", form.email))}>Gmail (OAuth)</button>
          <button type="button" className={btn} disabled={busy || !form.email} onClick={() => run(() => api.addOAuthAccount("microsoft", form.email))}>Outlook (OAuth)</button>
        </div>
      </form>
      {error && <p className="text-red-500">{error}</p>}

      <h2 className="font-semibold">Dossiers</h2>
      <div className="flex flex-wrap gap-2">
        {folders.map((f) => (
          <button key={f.id} className={btn} onClick={() => api.listMessages(f.id).then(setMessages)}>
            {f.name} ({f.unreadCount}/{f.totalCount})
          </button>
        ))}
      </div>

      <h2 className="font-semibold">Recherche plein texte</h2>
      <input className={input} placeholder="Rechercher…" value={query}
        onChange={(e) => { setQuery(e.target.value); api.searchEmails(e.target.value).then(setMessages).catch((er) => setError(String(er))); }} />
      <ul className="space-y-1">
        {messages.map((m) => (
          <li key={m.id} className={m.isRead ? "text-muted-foreground" : "font-medium"}>
            {m.fromName ?? m.fromAddr} — {m.subject || "(sans objet)"} <span className="text-xs">{m.date.slice(0, 10)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
