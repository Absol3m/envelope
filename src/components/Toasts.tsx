import { AlertCircle, Undo2 } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { useMail } from "@/lib/store";
import { cn } from "@/lib/utils";
import { Kbd } from "./ui/primitives";

/** Un seul toast à la fois, centré en bas. La barre fine (envoi différé) matérialise le temps d'annulation restant. */
export function Toasts() {
  const { toasts, dismissToast } = useMail(useShallow((s) => ({ toasts: s.toasts, dismissToast: s.dismissToast })));
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-6 z-[80] flex justify-center" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          role="status"
          className={cn(
            "anim-fade-up pointer-events-auto relative flex items-center gap-3 overflow-hidden rounded-xl py-2.5 pl-4 pr-2.5 text-[13px] font-medium shadow-pop",
            t.tone === "error" ? "bg-danger text-white" : "bg-fg text-bg",
          )}
        >
          {t.tone === "error" && <AlertCircle size={15} />}
          <span className="max-w-[460px] truncate">{t.message}</span>
          {t.onAction && (
            <button
              onClick={() => { t.onAction?.(); dismissToast(t.id); }}
              className="press flex items-center gap-1.5 rounded-lg bg-bg/15 px-2.5 py-1 hover:bg-bg/25"
            >
              <Undo2 size={13} /> {t.actionLabel} <Kbd className="border-bg/30 bg-transparent text-bg/80">Z</Kbd>
            </button>
          )}
          {t.countdown && <span className="absolute bottom-0 left-0 h-[2px] bg-bg/40" style={{ animation: `shrink ${t.duration}ms linear forwards` }} />}
        </div>
      ))}
    </div>
  );
}
