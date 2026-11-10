import type { DragEvent } from "react";
import { avatarBackground, displayName, initials } from "./format";
import { DRAG_TYPE } from "./views";
import type { ThreadSummary } from "./api";

/**
 * Glisser d'une conversation : le navigateur photographierait la ligne entière (à pleine largeur,
 * elle masquerait les dossiers visés). On lui donne à la place une pastille compacte (avatar + objet),
 * on estompe la ligne d'origine et on signale les dossiers qui acceptent le dépôt (CSS, `data-dragging`).
 * Le geste lui-même reste celui du moteur web de l'OS : aucun code ne tourne pendant le déplacement.
 */
/** Nettoyage du glisser en cours (un `dragend` perdu ne doit pas laisser la barre latérale éteinte). */
let endDrag: (() => void) | null = null;

export function startThreadDrag(e: DragEvent<HTMLElement>, t: ThreadSummary): void {
  endDrag?.();
  const row = e.currentTarget;
  e.dataTransfer.setData(DRAG_TYPE, String(t.id));
  e.dataTransfer.effectAllowed = "move";

  const label = displayName(t.fromName, t.fromAddr);
  const chip = document.createElement("div");
  chip.className = "drag-chip";
  const avatar = document.createElement("span");
  avatar.className = "drag-chip-avatar";
  avatar.textContent = initials(label);
  avatar.style.background = avatarBackground(t.fromName, t.fromAddr);
  const subject = document.createElement("span");
  subject.className = "drag-chip-subject";
  subject.textContent = t.subject || "(sans objet)";
  chip.append(avatar, subject);
  if (t.messageCount > 1) {
    const count = document.createElement("span");
    count.className = "drag-chip-count";
    count.textContent = String(t.messageCount);
    chip.append(count);
  }
  document.body.append(chip);
  e.dataTransfer.setDragImage(chip, 16, 17);
  setTimeout(() => chip.remove(), 0); // l'image est capturée de façon synchrone

  // Après la capture : estomper la ligne plus tôt la griserait aussi dans l'image.
  requestAnimationFrame(() => row.classList.add("thread-dragging"));
  document.body.dataset.dragging = "";

  // `dragend` n'arrive pas si la ligne a été retirée de la liste par le dépôt : on écoute aussi `drop`.
  const end = () => {
    endDrag = null;
    row.classList.remove("thread-dragging");
    delete document.body.dataset.dragging;
    window.removeEventListener("dragend", end);
    document.removeEventListener("drop", end, true);
  };
  window.addEventListener("dragend", end);
  document.addEventListener("drop", end, true);
  endDrag = end;
}
