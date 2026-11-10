/** Conventions propres à chaque système : macOS d'un côté, Windows et Linux de l'autre. */
export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);
export const isWindows = typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);

/** Nom de la fenêtre de réglages selon l'usage du système. */
export const SETTINGS_LABEL = isMac ? "Réglages" : "Paramètres";

const MAC_KEYS: Record<string, string> = { mod: "⌘", shift: "⇧", enter: "↵" };
const PC_KEYS: Record<string, string> = { mod: "Ctrl", shift: "Maj", enter: "Entrée" };

/** Touches d'un raccourci, écrit dans l'ordre d'affichage (`"shift+mod+z"` sur macOS,
 *  `"mod+shift+z"` ailleurs) : `mod` vaut ⌘ sur macOS et Ctrl ailleurs. */
export function keys(combo: string, mac = isMac): string[] {
  const names = mac ? MAC_KEYS : PC_KEYS;
  return combo.split("+").map((k) => names[k.toLowerCase()] ?? (k.length === 1 ? k.toUpperCase() : k));
}

/** Raccourci en une chaîne : « ⌘K » sur macOS, « Ctrl+K » ailleurs ; une touche seule est inchangée. */
export function shortcut(combo: string, mac = isMac): string {
  return keys(combo, mac).join(mac ? "" : "+");
}

/** Zone de glisser de la fenêtre : seulement sous macOS, où la barre de titre est fusionnée
 *  avec l'interface. Ailleurs la barre de titre native suffit, et glisser le contenu
 *  déplacerait la fenêtre à contre-usage. */
export const dragRegion: { "data-tauri-drag-region"?: true } = isMac ? { "data-tauri-drag-region": true } : {};

/** La cible d'un événement est-elle un champ où l'on écrit ? */
export function isEditable(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null; // pas d'instanceof : la cible peut venir de l'iframe du message
  return !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
}

/** Le menu contextuel de la vue web (Précédent, Actualiser, Imprimer…) trahit le navigateur :
 *  on ne le garde que là où il sert (champs de saisie, texte sélectionné), et en développement. */
export function suppressWebviewContextMenu(doc: Document) {
  if (import.meta.env.DEV) return;
  doc.addEventListener("contextmenu", (e) => {
    if (!isEditable(e.target) && !doc.getSelection()?.toString()) e.preventDefault();
  });
}
