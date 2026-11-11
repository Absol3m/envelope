import { pref } from "./pref";

/** Aperçu d'un nouveau message, tel qu'envoyé par le démon de synchro (contenu non fiable : texte brut). */
export interface FreshMail {
  from: string;
  subject: string;
}
export interface SyncedPayload {
  accountId: string;
  newMessages: number;
  fresh: FreshMail[];
  freshTotal: number;
}

export const notificationsEnabled = () => pref.get("notifications", "on") === "on";
export const setNotificationsEnabled = (on: boolean) => pref.set("notifications", on ? "on" : "off");

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** Titre et corps de la notification : un message → expéditeur + objet ; plusieurs → récapitulatif. */
export function notificationText(p: Pick<SyncedPayload, "fresh" | "freshTotal">): { title: string; body: string } | null {
  if (!p.freshTotal || !p.fresh.length) return null;
  if (p.freshTotal === 1) {
    const [m] = p.fresh;
    return { title: clip(m.from, 60), body: clip(m.subject.trim() || "(sans objet)", 120) };
  }
  const senders = [...new Set(p.fresh.map((m) => m.from))].map((s) => clip(s, 30));
  const more = p.freshTotal - senders.length;
  return { title: `${p.freshTotal} nouveaux messages`, body: clip(`${senders.join(", ")}${more > 0 ? ` et ${more} autre${more > 1 ? "s" : ""}` : ""}`, 140) };
}

/** Notification native (plugin officiel), seulement si l'application n'est pas au premier plan. */
export async function notifyNewMail(p: SyncedPayload) {
  const msg = notificationText(p);
  if (!msg || !notificationsEnabled() || document.hasFocus()) return;
  try {
    const { isPermissionGranted, requestPermission, sendNotification } = await import("@tauri-apps/plugin-notification");
    if (!(await isPermissionGranted()) && (await requestPermission()) !== "granted") return;
    sendNotification(msg);
  } catch {
    /* notifications indisponibles sur ce système : sans conséquence */
  }
}
