import * as Dialog from "@radix-ui/react-dialog";
import { Clock, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useComposer } from "@/lib/composer";
import { scheduledAt } from "@/lib/format";
import { useMail } from "@/lib/store";
import { btnPrimary } from "./ui/primitives";
import { t as tr } from "@/lib/i18n";
import { describeStored } from "@/lib/backendError";


/** Envois programmés en attente : envoyer tout de suite, ou retirer de la file pour modifier le message. */
export function ScheduledDialog() {
  const { scheduled, backend, setScheduledOpen, refreshScheduled, toast } = useMail(useShallow((s) => ({ scheduled: s.scheduled, backend: s.backend, setScheduledOpen: s.setScheduledOpen, refreshScheduled: s.refreshScheduled, toast: s.toast })));
  const [busy, setBusy] = useState<number | null>(null);

  useEffect(() => { void refreshScheduled(); }, [refreshScheduled]);

  const sendNow = async (id: number) => {
    setBusy(id);
    try {
      await backend.sendScheduledNow(id);
    } catch (e) {
      toast({ message: tr("scheduled:sendFailed", { error: String(e) }), tone: "error" });
    } finally {
      setBusy(null);
      void refreshScheduled();
    }
  };
  const edit = (id: number, accountId: string) => {
    setScheduledOpen(false);
    void useComposer.getState().editScheduled(id, accountId);
  };

  return (
    <Dialog.Root open onOpenChange={(o) => !o && setScheduledOpen(false)}>
      <Dialog.Portal>
        <Dialog.Overlay className="anim-fade-in fixed inset-0 z-[90] bg-black/35 backdrop-blur-[3px]" />
        <Dialog.Content aria-describedby={undefined} className="fixed left-1/2 top-1/2 z-[91] flex max-h-[min(560px,88vh)] w-[min(560px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col rounded-2xl bg-elevated p-6 shadow-pop">
          <div className="flex items-start gap-3">
            <span className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-accent-soft text-accent"><Clock size={18} /></span>
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-[16px] font-semibold tracking-tight">{tr("scheduled:title")}</Dialog.Title>
              <p className="mt-0.5 text-[12.5px] text-muted">{tr("scheduled:hint")}</p>
            </div>
            <Dialog.Close aria-label={tr("common:close")} className="press rounded-md p-1 text-muted hover:bg-hover hover:text-fg"><X size={16} /></Dialog.Close>
          </div>
          <ul className="mt-4 min-h-0 flex-1 divide-y divide-border overflow-y-auto rounded-xl border border-border">
            {scheduled.map((q) => (
              <li key={q.id} className="px-3 py-2.5 text-[13px]">
                <div className="flex items-baseline gap-2">
                  <span className="min-w-0 flex-1 truncate font-medium">{q.subject || tr("list:noSubject")}</span>
                  <span className="shrink-0 text-[12px] text-muted">{q.status === "sending" ? tr("scheduled:sending") : q.outbox ? tr("scheduled:outboxWaiting") : scheduledAt(q.sendAt * 1000)}</span>
                </div>
                <div className="truncate text-[12px] text-muted">{tr("scheduled:to", { list: q.to.join(", ") })}</div>
                {(q.status === "failed" || q.outbox) && q.error && <p className="mt-1 text-[12px] text-danger">{describeStored(q.error)}</p>}
                <div className="mt-2 flex gap-2">
                  <button className={`${btnPrimary} h-7 px-3 text-[12px]`} disabled={busy !== null || q.status === "sending"} onClick={() => void sendNow(q.id)}>{tr("scheduled:sendNow")}</button>
                  <button className="press h-7 rounded-lg px-3 text-[12px] font-medium text-accent hover:bg-hover disabled:opacity-50" disabled={busy !== null || q.status === "sending"} onClick={() => edit(q.id, q.accountId)}>{tr("scheduled:edit")}</button>
                </div>
              </li>
            ))}
          </ul>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
