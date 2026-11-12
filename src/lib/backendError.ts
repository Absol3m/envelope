import { i18next, t } from "./i18n";

/** Erreur du moteur Rust telle qu'elle traverse l'IPC (voir `src-tauri/src/error.rs`) : un code stable traduit ici,
 *  ses paramètres, et le message de repli (anglais, ou texte brut d'une bibliothèque ou du serveur distant). */
export interface WireError {
  code: string;
  args: Record<string, string>;
  message: string;
}

const isWire = (v: unknown): v is WireError =>
  typeof v === "object" && v !== null && typeof (v as WireError).code === "string" && typeof (v as WireError).message === "string";

/** Texte de l'erreur dans la langue active. Sans code (texte brut) ou sans traduction, le message du moteur. */
export function describeWire(w: WireError): string {
  if (!w.code) return w.message;
  const options: Record<string, unknown> = { ...w.args, defaultValue: w.message };
  if ("count" in w.args) options.count = Number(w.args.count); // choix du pluriel
  return t(`errors:rust.${w.code}`, options) as string;
}

/** Erreur du moteur levée par un appel IPC ; `String(e)` donne le texte traduit (lu à l'affichage, la langue peut changer). */
export class BackendError extends Error {
  constructor(readonly wire: WireError) {
    super();
    this.name = "BackendError";
    Object.defineProperty(this, "message", { get: () => describeWire(wire) });
  }
  override toString(): string {
    return this.message;
  }
}

/** Convertit ce que rejette `invoke` : une erreur du moteur devient une `BackendError`, tout le reste est inchangé. */
export const toBackendError = (e: unknown): unknown => (isWire(e) ? new BackendError(e) : e);

/** Erreur gardée en base ou émise dans un événement (JSON de `AppError::to_wire`) : traduite à l'affichage ;
 *  un texte ordinaire (ancienne valeur, détail technique) reste tel quel. */
export function describeStored(text: string | null | undefined): string | null {
  if (!text) return null;
  if (text.startsWith("{")) {
    try {
      const v: unknown = JSON.parse(text);
      if (isWire(v)) return describeWire(v);
    } catch {
      /* pas du JSON : texte brut */
    }
  }
  return text;
}

export { i18next };
