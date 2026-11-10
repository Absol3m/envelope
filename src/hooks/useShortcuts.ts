import { useEffect } from "react";
import { useComposer } from "@/lib/composer";
import { selectionIds, useMail } from "@/lib/store";
import { DESTINATIONS, destinationsFor, GO_VIEWS, isInboxView, isPurgeView } from "@/lib/views";
import { isEditable } from "@/lib/platform";
import type { Category } from "@/lib/categories";

const CATEGORIES: Category[] = ["all", "priority", "newsletters", "notifications"];

/** Raccourcis globaux, style Superhuman : J/K, E, S, U, C, R, A, F, Z, /, g+lettre, ⌘K. */
export function useShortcuts() {
  useEffect(() => {
    let goPending = false;
    let goTimer: ReturnType<typeof setTimeout> | undefined;

    const onKey = (e: KeyboardEvent) => {
      const st = useMail.getState();
      const typing = isEditable(e.target);

      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        st.setPalette(!st.paletteOpen);
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key === ",") {
        e.preventDefault();
        st.openSettings("accounts");
        return;
      }
      const blocked = st.paletteOpen || st.settingsTab || st.helpOpen || useComposer.getState().draft;
      if (!typing && !blocked && (e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "a") {
        e.preventDefault();
        st.checkAll();
        return;
      }
      if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (blocked) return;

      if (goPending) {
        goPending = false;
        clearTimeout(goTimer);
        const v = GO_VIEWS[e.key.toLowerCase()];
        if (v) {
          e.preventDefault();
          st.setView(v);
        }
        return;
      }

      const ids = selectionIds(st); // la sélection multiple, à défaut la conversation ouverte
      switch (e.key) {
        case "j": case "ArrowDown": e.preventDefault(); st.move(1); break;
        case "k": case "ArrowUp": e.preventDefault(); st.move(-1); break;
        case "Delete": case "Backspace": if (ids.length > 0 && isPurgeView(st.view)) { e.preventDefault(); void st.deleteForever(ids); } break;
        case "s": if (ids.length > 0) st.toggleStar(ids); break;
        case "u": if (ids.length > 0) st.toggleRead(ids); break;
        case "c": e.preventDefault(); useComposer.getState().compose(); break;
        case "r": e.preventDefault(); useComposer.getState().respond("reply"); break;
        case "a": e.preventDefault(); useComposer.getState().respond("replyAll"); break;
        case "f": e.preventDefault(); useComposer.getState().respond("forward"); break;
        case "z": useComposer.getState().undoSend(); break;
        case "/": e.preventDefault(); window.dispatchEvent(new Event("envelope:focus-search")); break;
        case "?": st.setHelp(true); break;
        case "Escape": if (st.checked.size > 0) st.clearChecked(); else if (st.selectedId !== null) st.select(null); break;
        case "g":
          goPending = true;
          goTimer = setTimeout(() => (goPending = false), 900);
          break;
        default: {
          const to = destinationsFor(st.view).find((d) => DESTINATIONS[d].key === e.key.toUpperCase());
          if (to) {
            if (ids.length > 0) st.moveTo(ids, to);
          } else if (isInboxView(st.view) && /^[1-4]$/.test(e.key)) st.setCategory(CATEGORIES[Number(e.key) - 1]);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      clearTimeout(goTimer);
    };
  }, []);
}
