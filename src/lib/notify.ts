import { pref } from "./pref";
import { t } from "./i18n";

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
export function notificationText(p: Pick<SyncedPayload, "fresh" | "freshTotal">, hidden = false): { title: string; body: string } | null {
  if (!p.freshTotal) return null;
  // Application verrouillée : ni expéditeur ni objet sur l'écran (le moteur n'en envoie d'ailleurs pas).
  if (hidden) return { title: "Envelope", body: t("notify:newMessages", { count: p.freshTotal }) };
  if (!p.fresh.length) return null;
  if (p.freshTotal === 1) {
    const [m] = p.fresh;
    return { title: clip(m.from, 60), body: clip(m.subject.trim() || t("list:noSubject"), 120) };
  }
  const senders = [...new Set(p.fresh.map((m) => m.from))].map((s) => clip(s, 30));
  const more = p.freshTotal - senders.length;
  return { title: t("notify:newMessages", { count: p.freshTotal }), body: clip(more > 0 ? t("notify:withOthers", { names: senders.join(", "), count: more }) : senders.join(", "), 140) };
}

/** Envoie une notification native (plugin officiel) ; lève une erreur si le système la refuse. */
export async function sendNative(msg: { title: string; body: string }) {
  const { isPermissionGranted, requestPermission, sendNotification } = await import("@tauri-apps/plugin-notification");
  if (!(await isPermissionGranted()) && (await requestPermission()) !== "granted") throw new Error(t("notify:denied"));
  sendNotification(msg);
}

/** Nouveau message : notification seulement si l'application n'est pas au premier plan. */
export async function notifyNewMail(p: SyncedPayload, locked = false) {
  const msg = notificationText(p, locked);
  if (!msg || !notificationsEnabled() || document.hasFocus()) return;
  try {
    await sendNative(msg);
  } catch (e) {
    console.warn("notification impossible :", e); // sans conséquence pour l'utilisateur
  }
}
