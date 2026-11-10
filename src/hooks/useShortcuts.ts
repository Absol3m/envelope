import { useEffect } from "react";
import { useComposer } from "@/lib/composer";
import { useMail } from "@/lib/store";
import { GO_VIEWS, isInboxView } from "@/lib/views";
import type { Category } from "@/lib/categories";

const CATEGORIES: Category[] = ["all", "priority", "newsletters", "notifications"];

/** Raccourcis globaux, style Superhuman : J/K, E, S, U, C, R, A, F, Z, /, g+lettre, ⌘K. */
export function useShortcuts() {
  useEffect(() => {
    let goPending = false;
    let goTimer: ReturnType<typeof setTimeout> | undefined;

    const onKey = (e: KeyboardEvent) => {
      const st = useMail.getState();
      const el = e.target as HTMLElement | null;
      const typing = !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);

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
      if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (st.paletteOpen || st.settingsTab || st.helpOpen || useComposer.getState().draft) return;

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

      const id = st.selectedId;
      switch (e.key) {
        case "j": case "ArrowDown": e.preventDefault(); st.move(1); break;
        case "k": case "ArrowUp": e.preventDefault(); st.move(-1); break;
        case "e": if (id !== null) st.archive(id); break;
        case "s": if (id !== null) st.toggleStar(id); break;
        case "u": if (id !== null) st.toggleRead(id); break;
        case "c": e.preventDefault(); useComposer.getState().compose(); break;
        case "r": e.preventDefault(); useComposer.getState().respond("reply"); break;
        case "a": e.preventDefault(); useComposer.getState().respond("replyAll"); break;
        case "f": e.preventDefault(); useComposer.getState().respond("forward"); break;
        case "z": if (!useComposer.getState().undoSend()) st.undo(); break;
        case "/": e.preventDefault(); window.dispatchEvent(new Event("envelope:focus-search")); break;
        case "?": st.setHelp(true); break;
        case "Escape": if (id !== null) st.select(null); break;
        case "g":
          goPending = true;
          goTimer = setTimeout(() => (goPending = false), 900);
          break;
        default:
          if (isInboxView(st.view) && /^[1-4]$/.test(e.key)) st.setCategory(CATEGORIES[Number(e.key) - 1]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      clearTimeout(goTimer);
    };
  }, []);
}
