import { Lock } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useMail } from "@/lib/store";
import { dragRegion } from "@/lib/platform";
import { errText } from "@/lib/utils";
import { btnPrimary, inputCls } from "./ui/primitives";

/** Écran de déverrouillage : seul écran affiché tant que l'application est verrouillée (le moteur refuse le reste). */
export function LockScreen() {
  const backend = useMail((s) => s.backend);
  const method = useMail((s) => s.lock?.method);
  const unlocked = useMail((s) => s.unlocked);
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const field = useRef<HTMLInputElement>(null);

  const attempt = async (job: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await job();
      setPassword("");
      await unlocked();
    } catch (e) {
      setError(errText(e));
      setPassword("");
    } finally {
      setBusy(false);
    }
  };
  const unlockSystem = () => void attempt(() => backend.unlockSystem());

  // Par le système : l'invite s'ouvre d'elle-même à l'affichage de l'écran ; par mot de passe : le champ prend le focus.
  useEffect(() => {
    if (method === "system") void attempt(() => backend.unlockSystem());
    else field.current?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [method]);

  // Secours : mot de passe oublié, trousseau réinitialisé, authentification du système retirée.
  const reset = async () => {
    const ok = await backend.confirm("Réinitialiser Envelope efface TOUTES les données de cet ordinateur : comptes, messages archivés en local, modèles et réglages. Les e-mails restent sur vos serveurs, mais ce qui n'existe qu'ici est perdu.\n\nÀ n'utiliser que si vous ne pouvez plus déverrouiller Envelope. Continuer ?", "Tout effacer");
    if (ok) await attempt(() => backend.resetApp());
  };

  return (
    <main className="flex h-full flex-col items-center justify-center gap-5 bg-bg px-6" {...dragRegion} aria-label="Application verrouillée">
      <span className="inline-flex h-14 w-14 items-center justify-center rounded-2xl bg-accent-soft text-accent"><Lock size={26} /></span>
      <div className="text-center">
        <h1 className="text-[18px] font-semibold tracking-tight">Envelope est verrouillé</h1>
        <p className="mt-1 text-[13px] text-muted">{method === "password" ? "Saisissez votre mot de passe pour continuer." : "Authentifiez-vous pour continuer."}</p>
      </div>
      {method === "password" ? (
        <form className="flex w-[min(320px,100%)] flex-col gap-2.5" onSubmit={(e) => { e.preventDefault(); if (password) void attempt(() => backend.unlockPassword(password)); }}>
          <input ref={field} type="password" aria-label="Mot de passe" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} disabled={busy} className={inputCls} />
          <button type="submit" disabled={busy || !password} className={btnPrimary}>Déverrouiller</button>
        </form>
      ) : (
        <button type="button" onClick={unlockSystem} disabled={busy} className={btnPrimary}>Déverrouiller</button>
      )}
      <button type="button" onClick={() => void reset()} disabled={busy} className="press text-[12px] text-muted underline-offset-2 hover:text-fg hover:underline">Impossible de déverrouiller ?</button>
      {error && <p role="alert" className="max-w-[320px] text-center text-[12.5px] text-danger">{error}</p>}
    </main>
  );
}
