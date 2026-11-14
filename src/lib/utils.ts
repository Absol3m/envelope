import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Fusionne des classes Tailwind sans conflits (convention shadcn/ui). */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Ouvre une URL http(s) ou mailto dans le navigateur système (jamais dans l'application). */
export async function openExternal(url: string) {
  if (!/^(https?:|mailto:)/i.test(url)) return;
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

/** Message d'une erreur venue du moteur, sans préfixes techniques (« Error: », « authentification : »). */
export const errText = (e: unknown) => String(e).replace(/^Error: /, "").replace(/^authentification : /, "");

const EMAIL_RE = /^[^\s@<>,;"]+@[^\s@<>,;"]+\.[^\s@<>,;"]{2,}$/;
/** Adresse plausible (le moteur fait foi à l'envoi). */
export const isEmail = (s: string) => EMAIL_RE.test(s.trim());
/** Même adresse, casse ignorée. */
export const sameAddr = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
