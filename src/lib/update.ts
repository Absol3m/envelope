import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { isTauri } from "./api";
import { pref } from "./pref";
import { errText } from "./utils";

interface UpdateState {
  /** Numéro de la version plus récente publiée (src-tauri/src/update.rs). */
  available: string | null;
  status: "idle" | "checking" | "installing";
  /** Dernière vérification aboutie (ISO) ; `null` : pas encore vérifié pendant cette session. */
  checkedAt: string | null;
  error: string | null;
}

export const useUpdate = create<UpdateState>(() => ({ available: null, status: "idle", checkedAt: null, error: null }));

/** Vérification automatique (au démarrage puis chaque jour) : activée par défaut, désactivable dans les réglages. */
export const autoUpdateEnabled = () => pref.get("autoUpdate", "on") === "on";
export const setAutoUpdateEnabled = (on: boolean) => pref.set("autoUpdate", on ? "on" : "off");

/** Interroge le canal de mise à jour (GitHub). Hors de l'application (aperçu navigateur) : rien à vérifier. */
export async function checkForUpdate(): Promise<void> {
  if (!isTauri() || useUpdate.getState().status !== "idle") return;
  useUpdate.setState({ status: "checking", error: null });
  try {
    const available = await invoke<string | null>("check_update");
    useUpdate.setState({ available, status: "idle", checkedAt: new Date().toISOString() });
  } catch (e) {
    useUpdate.setState({ status: "idle", error: errText(e) });
  }
}

/** Télécharge et installe la mise à jour ; l'application redémarre d'elle-même si tout se passe bien. */
export async function installUpdate(): Promise<void> {
  if (useUpdate.getState().status !== "idle") return;
  useUpdate.setState({ status: "installing", error: null });
  try {
    await invoke("install_update");
  } catch (e) {
    useUpdate.setState({ status: "idle", error: errText(e) });
  }
}

const FIRST_CHECK_MS = 15_000; // après le démarrage, pour ne pas concurrencer la synchro
const DAY_MS = 24 * 60 * 60 * 1000;

/** Lance la vérification automatique ; rend la fonction d'arrêt. */
export function startUpdateChecks(): () => void {
  if (!isTauri()) return () => {};
  const run = () => void (autoUpdateEnabled() && checkForUpdate());
  const first = setTimeout(run, FIRST_CHECK_MS);
  const daily = setInterval(run, DAY_MS);
  return () => {
    clearTimeout(first);
    clearInterval(daily);
  };
}
