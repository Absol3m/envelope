import { useEffect } from "react";
import { useMail } from "./store";

const CHECK_MS = 10_000;
const ACTIVITY = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart"] as const;

/** Verrouille l'application après `timeoutSecs` d'inactivité (aucune frappe ni mouvement) : 0 = jamais. */
export function useAutoLock(timeoutSecs: number, active: boolean) {
  useEffect(() => {
    if (!active || timeoutSecs <= 0) return;
    let last = Date.now();
    const touch = () => { last = Date.now(); };
    const check = () => { if (Date.now() - last >= timeoutSecs * 1000) void useMail.getState().lockNow(); };
    // Fenêtre masquée ou en veille, les minuteries du navigateur sont ralenties : on revérifie dès le retour.
    const onVisible = () => { if (document.visibilityState === "visible") check(); };
    ACTIVITY.forEach((e) => window.addEventListener(e, touch, { passive: true, capture: true }));
    document.addEventListener("visibilitychange", onVisible);
    const timer = setInterval(check, CHECK_MS);
    return () => {
      ACTIVITY.forEach((e) => window.removeEventListener(e, touch, { capture: true }));
      document.removeEventListener("visibilitychange", onVisible);
      clearInterval(timer);
    };
  }, [timeoutSecs, active]);
}
