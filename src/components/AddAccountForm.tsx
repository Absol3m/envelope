import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useMail } from "@/lib/store";
import { cn } from "@/lib/utils";
import { btnGhost, btnPrimary, inputCls, Spinner } from "./ui/primitives";

/** Ajout d'un compte (mot de passe d'application ou OAuth) : un seul formulaire pour l'accueil et les réglages. */
export function AddAccountForm({ large = false }: { large?: boolean }) {
  const { backend, reload } = useMail(useShallow((s) => ({ backend: s.backend, reload: s.reload })));
  const [form, setForm] = useState({ email: "", password: "" });
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (key: string, fn: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    try {
      await fn();
      setForm({ email: "", password: "" });
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };
  const email = form.email.trim();
  const field = cn(inputCls, large && "h-10 text-[13.5px]");
  const oauth = (
    <>
      <button type="button" className={btnGhost} disabled={busy !== null || !email} onClick={() => void run("g", () => backend.addOAuthAccount("google", email))}>Gmail{large ? "" : " (OAuth)"}</button>
      <button type="button" className={btnGhost} disabled={busy !== null || !email} onClick={() => void run("m", () => backend.addOAuthAccount("microsoft", email))}>Outlook{large ? "" : " (OAuth)"}</button>
    </>
  );

  return (
    <form className={cn("space-y-2.5", !large && "max-w-[420px]")} onSubmit={(e) => { e.preventDefault(); void run("add", () => backend.addAccount({ email, password: form.password })); }}>
      <input className={field} type="email" autoFocus={large} autoComplete="off" placeholder="Adresse e-mail" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} />
      <input className={field} type="password" autoComplete="off" placeholder="Mot de passe d'application" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} />
      <button className={cn(btnPrimary, large ? "h-10 w-full" : "")} disabled={busy !== null || !email || !form.password}>
        {busy === "add" && <Spinner />}
        {large ? "Se connecter" : "Ajouter"}
      </button>
      {large ? <div className="grid grid-cols-2 gap-2">{oauth}</div> : <div className="flex flex-wrap gap-2">{oauth}</div>}
      {error && <p className="selectable rounded-lg bg-danger/10 px-3 py-2 text-[12.5px] text-danger">{error}</p>}
    </form>
  );
}
