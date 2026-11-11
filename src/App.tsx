import { lazy, Suspense, useEffect } from "react";
import { useShallow } from "zustand/react/shallow";
import { Reader } from "@/components/Reader";
import { Sidebar } from "@/components/Sidebar";
import { TopBar } from "@/components/TopBar";
import { ThreadList } from "@/components/ThreadList";
import { Toasts } from "@/components/Toasts";
import { TooltipProvider } from "@/components/ui/primitives";
import { useShortcuts } from "@/hooks/useShortcuts";
import { isTauri } from "@/lib/api";
import { useAutoLock } from "@/lib/autolock";
import { useComposer } from "@/lib/composer";
import { notifyNewMail, type SyncedPayload } from "@/lib/notify";
import { useMail } from "@/lib/store";
import { dragRegion } from "@/lib/platform";

// Écrans rarement ouverts : chargés à la demande, hors du paquet initial.
const Onboarding = lazy(() => import("@/components/Onboarding").then((m) => ({ default: m.Onboarding })));
const Composer = lazy(() => import("@/components/Composer").then((m) => ({ default: m.Composer })));
const Palette = lazy(() => import("@/components/Palette").then((m) => ({ default: m.Palette })));
const Settings = lazy(() => import("@/components/Settings").then((m) => ({ default: m.Settings })));
const LockScreen = lazy(() => import("@/components/LockScreen").then((m) => ({ default: m.LockScreen })));
const ScheduledDialog = lazy(() => import("@/components/ScheduledDialog").then((m) => ({ default: m.ScheduledDialog })));
const ImportDialog = lazy(() => import("@/components/ImportDialog").then((m) => ({ default: m.ImportDialog })));
const ShortcutsHelp = lazy(() => import("@/components/Shortcuts").then((m) => ({ default: m.ShortcutsHelp })));

export default function App() {
  const { ready, needsOnboarding, density, paletteOpen, settingsOpen, helpOpen, importOpen, scheduledOpen, locked, autoLockSecs, lockEnabled } = useMail(
    useShallow((s) => ({ locked: s.lock?.locked === true, autoLockSecs: s.lock?.timeoutSecs ?? 0, lockEnabled: (s.lock?.method ?? "off") !== "off", scheduledOpen: s.scheduledOpen && s.scheduled.length > 0, importOpen: s.importOpen, ready: s.ready, needsOnboarding: s.needsOnboarding, density: s.density, paletteOpen: s.paletteOpen, settingsOpen: s.settingsTab !== null, helpOpen: s.helpOpen })),
  );
  const composing = useComposer((s) => s.draft !== null);
  useShortcuts();

  useEffect(() => {
    void useMail.getState().init();
    if (!isTauri()) return;
    // Événements du démon de synchro : la liste se met à jour sans geste de l'utilisateur.
    const unlisten: Promise<() => void>[] = [];
    import("@tauri-apps/api/event").then(({ listen }) => {
      unlisten.push(listen<SyncedPayload>("mail:synced", (e) => {
        const locked = useMail.getState().lock?.locked === true;
        if (!locked) void useMail.getState().reload({ silent: true }); // verrouillé, le moteur refuserait
        void notifyNewMail(e.payload, locked);
      }));
      unlisten.push(listen<{ accountId: string; remaining: number }>("mail:backfill", (e) => useMail.getState().setBackfill(e.payload.accountId, e.payload.remaining)));
      unlisten.push(listen("mail:status", () => { if (useMail.getState().lock?.locked !== true) void useMail.getState().refreshAccounts(); }));
      unlisten.push(listen<{ subject: string; error: string | null } | null>("scheduled:changed", (e) => {
        const { toast, refreshScheduled } = useMail.getState();
        void refreshScheduled();
        if (e.payload) toast(e.payload.error ? { message: `Envoi programmé impossible — ${e.payload.subject || "sans objet"} : ${e.payload.error}`, tone: "error", duration: 8000 } : { message: `Message programmé envoyé — ${e.payload.subject || "sans objet"}` });
      }));
    });
    return () => void unlisten.forEach((p) => p.then((fn) => fn()));
  }, []);

  useAutoLock(autoLockSecs, lockEnabled && !locked);

  if (!ready) return <div className="h-full bg-bg" {...dragRegion} />;
  if (locked) return <Suspense fallback={null}><LockScreen /></Suspense>;

  return (
    <TooltipProvider delayDuration={350}>
      {needsOnboarding ? (
        <Suspense fallback={null}><Onboarding /></Suspense>
      ) : (
        <div
          data-density={density}
          className="grid h-full grid-cols-[232px_minmax(340px,420px)_minmax(0,1fr)] grid-rows-[auto_minmax(0,1fr)] max-[1100px]:grid-cols-[60px_minmax(320px,380px)_minmax(0,1fr)]"
        >
          <Sidebar />
          <TopBar />
          <ThreadList />
          <Reader />
        </div>
      )}
      <Suspense fallback={null}>
        {composing && <Composer />}
        {paletteOpen && <Palette />}
        {settingsOpen && <Settings />}
        {helpOpen && <ShortcutsHelp />}
        {importOpen && <ImportDialog />}
        {scheduledOpen && <ScheduledDialog />}
      </Suspense>
      <Toasts />
    </TooltipProvider>
  );
}
