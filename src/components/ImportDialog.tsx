import * as Dialog from "@radix-ui/react-dialog";
import { FolderOpen, HardDrive, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { ImportFolder, ImportProgress, ImportReport } from "@/lib/api";
import { fileSize, formatNumber } from "@/lib/format";
import { useMail } from "@/lib/store";
import { btnPrimary, DialogFrame } from "./ui/primitives";
import { t as tr } from "@/lib/i18n";

type Step = "choose" | "pick" | "running" | "done";

const sizeLabel = (f: ImportFolder) => (f.kind === "maildir" ? tr("import:files", { count: f.size, n: formatNumber(f.size) }) : fileSize(f.size));

/** Importe des dossiers locaux d'un autre client (Thunderbird : mbox ou Maildir) dans « Sur cet ordinateur ». */
export function ImportDialog() {
  const { backend, setImportOpen, reload } = useMail(useShallow((s) => ({ backend: s.backend, setImportOpen: s.setImportOpen, reload: s.reload })));
  const [step, setStep] = useState<Step>("choose");
  const [source, setSource] = useState<{ token: string; path: string } | null>(null);
  const [folders, setFolders] = useState<ImportFolder[]>([]);
  const [checked, setChecked] = useState<ReadonlySet<string>>(new Set());
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Avancement émis par le moteur pendant l'import.
  useEffect(() => {
    if (step !== "running") return;
    let off: (() => void) | undefined;
    let stopped = false;
    void import("@tauri-apps/api/event").then(({ listen }) => listen<ImportProgress>("import:progress", (e) => setProgress(e.payload))).then((fn) => (stopped ? fn() : (off = fn)));
    return () => { stopped = true; off?.(); };
  }, [step]);

  const choose = async () => {
    setError(null);
    try {
      // Le sélecteur est ouvert par l'application ; on ne reçoit qu'un jeton, jamais un chemin à rendre.
      const picked = await backend.importPick();
      if (!picked) return;
      if (picked.folders.length === 0) return setError(tr("import:noFolders"));
      setSource({ token: picked.token, path: picked.path });
      setFolders(picked.folders);
      setChecked(new Set(picked.folders.filter((f) => f.selected).map((f) => f.key)));
      setStep("pick");
    } catch (e) {
      setError(String(e));
    }
  };

  const run = async () => {
    setError(null);
    setProgress(null);
    setStep("running");
    try {
      setReport(await backend.importRun(source!.token, [...checked]));
      setStep("done");
      void reload({ silent: true });
    } catch (e) {
      setError(String(e));
      setStep("pick");
      void reload({ silent: true }); // une partie a pu être importée avant l'erreur
    }
  };

  const toggle = (key: string) => setChecked((s) => { const n = new Set(s); n.has(key) ? n.delete(key) : n.add(key); return n; });
  const allOn = useMemo(() => folders.length > 0 && checked.size === folders.length, [folders, checked]);
  const busy = step === "running";

  return (
    <DialogFrame onOpenChange={(o) => !o && !busy && setImportOpen(false)} className="flex max-h-[min(640px,88vh)] w-[min(560px,94vw)] flex-col p-6" onInteractOutside={(e) => busy && e.preventDefault()} onEscapeKeyDown={(e) => busy && e.preventDefault()}>
      <div className="flex items-start gap-3">
        <span className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-accent-soft text-accent"><HardDrive size={18} /></span>
        <div className="min-w-0 flex-1">
          <Dialog.Title className="text-[16px] font-semibold tracking-tight">{tr("import:title")}</Dialog.Title>
          <p className="mt-0.5 text-[12.5px] text-muted">{tr("import:subtitle")}</p>
        </div>
        {!busy && <Dialog.Close aria-label={tr("common:close")} className="press rounded-md p-1 text-muted hover:bg-hover hover:text-fg"><X size={16} /></Dialog.Close>}
      </div>

      {step === "choose" && (
        <div className="mt-5">
          <p className="text-[13px] text-muted">{tr("import:choose")}</p>
          <button className={`${btnPrimary} mt-4`} onClick={() => void choose()}><FolderOpen size={15} />{tr("import:pickFolder")}</button>
        </div>
      )}

      {step === "pick" && (
        <>
          <p className="mt-4 truncate text-[12px] text-faint" title={source?.path}>{source?.path}</p>
          <div className="mt-2 flex items-center justify-between text-[12.5px]">
            <button className="press rounded-md px-1.5 py-0.5 font-medium text-accent hover:bg-hover" onClick={() => setChecked(allOn ? new Set() : new Set(folders.map((f) => f.key)))}>{allOn ? tr("import:deselectAll") : tr("import:selectAll")}</button>
            <span className="text-muted">{tr("import:selectedOf", { checked: checked.size, count: folders.length })}</span>
          </div>
          <ul className="mt-2 min-h-0 flex-1 divide-y divide-border overflow-y-auto rounded-xl border border-border">
            {folders.map((f) => (
              <li key={f.key}>
                <label className="flex cursor-default items-center gap-3 px-3 py-2 text-[13px] hover:bg-hover">
                  <input type="checkbox" checked={checked.has(f.key)} onChange={() => toggle(f.key)} className="h-4 w-4 accent-[var(--accent)]" />
                  <span className="min-w-0 flex-1 truncate">{f.name}</span>
                  <span className="tnum shrink-0 text-[12px] text-faint">{sizeLabel(f)}</span>
                </label>
              </li>
            ))}
          </ul>
          <div className="mt-4 flex items-center justify-end gap-2">
            <button className="press h-9 rounded-lg px-3 text-[13px] font-medium text-muted hover:bg-hover hover:text-fg" onClick={() => setStep("choose")}>{tr("import:change")}</button>
            <button className={btnPrimary} disabled={checked.size === 0} onClick={() => void run()}>{tr("import:importN", { count: checked.size })}</button>
          </div>
        </>
      )}

      {step === "running" && (
        <div className="mt-6" aria-live="polite">
          <p className="truncate text-[13px] font-medium">{progress?.folder ? progress.folder : tr("import:preparing")}</p>
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-hover" role="progressbar" aria-valuemin={0} aria-valuemax={progress?.foldersTotal ?? 1} aria-valuenow={progress?.foldersDone ?? 0}>
            <div className="h-full rounded-full bg-accent transition-[width] duration-300" style={{ width: `${progress && progress.foldersTotal > 0 ? Math.max(4, (progress.foldersDone / progress.foldersTotal) * 100) : 4}%` }} />
          </div>
          <p className="tnum mt-2 text-[12.5px] text-muted">
            {progress ? tr("import:folderOf", { n: Math.min(progress.foldersDone + 1, progress.foldersTotal), total: progress.foldersTotal }) : ""}
            {tr("import:importedSoFar", { count: progress?.imported ?? 0, n: formatNumber(progress?.imported ?? 0) })}
          </p>
          <p className="mt-3 text-[12px] text-faint">{tr("import:keepUsing")}</p>
        </div>
      )}

      {step === "done" && report && (
        <div className="mt-5">
          <p className="text-[14px] font-semibold">{tr("import:imported", { count: report.imported, n: formatNumber(report.imported) })}</p>
          <ul className="mt-2 space-y-0.5 text-[12.5px] text-muted">
            {report.duplicates > 0 && <li>{tr("import:duplicates", { count: report.duplicates, n: formatNumber(report.duplicates) })}</li>}
            {report.deleted > 0 && <li>{tr("import:deleted", { count: report.deleted, n: formatNumber(report.deleted) })}</li>}
            {report.failed > 0 && <li className="text-danger">{tr("import:failed", { count: report.failed, n: formatNumber(report.failed) })}</li>}
          </ul>
          <div className="mt-5 flex justify-end"><button className={btnPrimary} onClick={() => setImportOpen(false)}>{tr("import:done")}</button></div>
        </div>
      )}

      {error && <p role="alert" className="mt-4 rounded-lg bg-danger/10 px-3 py-2 text-[12.5px] text-danger">{error}</p>}
    </DialogFrame>
  );
}
