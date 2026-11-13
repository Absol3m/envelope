import { Lock } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useMail } from "@/lib/store";
import { dragRegion } from "@/lib/platform";
import { errText } from "@/lib/utils";
import { btnPrimary, inputCls } from "./ui/primitives";
import { t as tr } from "@/lib/i18n";

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

  // Par mot de passe : le champ prend le focus. Par le système : l'invite s'ouvre d'elle-même, mais seulement quand
  // Envelope est au premier plan. Verrouillée en arrière-plan (inactivité), l'app attend qu'on y revienne : l'invite du
  // système passe par-dessus toutes les fenêtres et surgirait au milieu d'une autre application.
  useEffect(() => {
    if (method !== "system") return void field.current?.focus();
    if (document.hasFocus()) return unlockSystem();
    window.addEventListener("focus", unlockSystem, { once: true });
    return () => window.removeEventListener("focus", unlockSystem);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [method]);

  // Secours : mot de passe oublié, trousseau réinitialisé, authentification du système retirée.
  const reset = async () => {
    const ok = await backend.confirm(tr("lock:resetConfirm"), tr("lock:eraseAll"));
    if (ok) await attempt(() => backend.resetApp());
  };

  return (
    <main className="flex h-full flex-col items-center justify-center gap-5 bg-bg px-6" {...dragRegion} aria-label={tr("lock:locked")}>
      <span className="inline-flex h-14 w-14 items-center justify-center rounded-2xl bg-accent-soft text-accent"><Lock size={26} /></span>
      <div className="text-center">
        <h1 className="text-[18px] font-semibold tracking-tight">{tr("lock:title")}</h1>
        <p className="mt-1 text-[13px] text-muted">{method === "password" ? tr("lock:enterPassword") : tr("lock:authenticate")}</p>
      </div>
      {method === "password" ? (
        <form className="flex w-[min(320px,100%)] flex-col gap-2.5" onSubmit={(e) => { e.preventDefault(); if (password) void attempt(() => backend.unlockPassword(password)); }}>
          <input ref={field} type="password" aria-label={tr("lock:password")} autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} disabled={busy} className={inputCls} />
          <button type="submit" disabled={busy || !password} className={btnPrimary}>{tr("lock:unlock")}</button>
        </form>
      ) : (
        <button type="button" onClick={unlockSystem} disabled={busy} className={btnPrimary}>{tr("lock:unlock")}</button>
      )}
      <button type="button" onClick={() => void reset()} disabled={busy} className="press text-[12px] text-muted underline-offset-2 hover:text-fg hover:underline">{tr("lock:cantUnlock")}</button>
      {error && <p role="alert" className="max-w-[320px] text-center text-[12.5px] text-danger">{error}</p>}
    </main>
  );
}
