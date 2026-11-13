import * as Dialog from "@radix-ui/react-dialog";
import { Download, ExternalLink, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { DetailAttachment } from "@/lib/api";
import { t as tr } from "@/lib/i18n";
import { useMail } from "@/lib/store";

/** Types dont l'aperçu est proposé (les mêmes que côté moteur) ; le reste s'ouvre dans l'application du système. */
export const previewable = (mime: string) => /^image\/(png|jpeg|gif|webp|avif|bmp|svg\+xml)$/i.test(mime);

/** Aperçu d'une image jointe, avec enregistrement et ouverture dans l'application du système. */
export function AttachmentPreview({ item, onClose, onSave, onOpen }: { item: DetailAttachment; onClose(): void; onSave(): void; onOpen(): void }) {
  const backend = useMail((s) => s.backend);
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    backend.attachmentPreview(item.id).then((s) => live && setSrc(s), () => live && (onClose(), onOpen())); // trop gros, illisible : on laisse le système l'ouvrir
    return () => { live = false; };
  }, [backend, item.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const name = item.filename ?? tr("reader:attachment");
  const btn = "press inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12.5px] text-muted hover:bg-hover hover:text-fg";
  return (
    <Dialog.Root open onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="anim-fade-in fixed inset-0 z-[90] bg-black/55 backdrop-blur-[3px]" />
        <Dialog.Content aria-describedby={undefined} className="fixed left-1/2 top-1/2 z-[91] flex max-h-[90vh] w-[min(900px,94vw)] -translate-x-1/2 -translate-y-1/2 flex-col rounded-2xl bg-elevated p-4 shadow-pop">
          <div className="flex items-center gap-2">
            <Dialog.Title className="min-w-0 flex-1 truncate text-[14px] font-semibold">{name}</Dialog.Title>
            <button onClick={onSave} className={btn}><Download size={14} />{tr("reader:save")}</button>
            <button onClick={onOpen} className={btn}><ExternalLink size={14} />{tr("reader:openWith")}</button>
            <Dialog.Close aria-label={tr("common:close")} className="press rounded-md p-1 text-muted hover:bg-hover hover:text-fg"><X size={16} /></Dialog.Close>
          </div>
          <div className="mt-3 flex min-h-[160px] flex-1 items-center justify-center overflow-auto rounded-xl bg-bg">
            {src && <img src={src} alt={name} className="max-h-[78vh] max-w-full object-contain" />}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** Image jointe affichée dans le fil, sous le message : chargée quand elle approche de l'écran, hauteur bornée.
 *  `onFail` : trop grosse ou illisible, l'appelant la garde comme simple pièce jointe. */
export function InlineImage({ item, onOpen, onSave, onFail }: { item: DetailAttachment; onOpen(): void; onSave(): void; onFail(): void }) {
  const backend = useMail((s) => s.backend);
  const box = useRef<HTMLDivElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    let live = true;
    const io = new IntersectionObserver(([e]) => {
      if (!e.isIntersecting) return;
      io.disconnect();
      backend.attachmentPreview(item.id).then((s) => live && setSrc(s), () => live && onFail());
    }, { rootMargin: "300px" });
    io.observe(el);
    return () => { live = false; io.disconnect(); };
  }, [backend, item.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const name = item.filename ?? tr("reader:attachment");
  return (
    <div ref={box} className="group relative inline-block max-w-full overflow-hidden rounded-xl bg-bg shadow-card" style={src ? undefined : { width: 240, height: 160 }}>
      {src && <button onClick={onOpen} className="block" aria-label={name}><img src={src} alt={name} className="block max-h-[360px] max-w-full object-contain" /></button>}
      <div className="absolute right-2 top-2 flex gap-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
        <button onClick={onSave} aria-label={tr("reader:save")} title={tr("reader:save")} className="press rounded-md bg-elevated/90 p-1.5 text-fg shadow-card backdrop-blur hover:bg-elevated"><Download size={14} /></button>
      </div>
    </div>
  );
}
