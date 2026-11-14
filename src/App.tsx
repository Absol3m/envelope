import { lazy, Suspense, useEffect } from "react";
import { loadComposer } from "@/lib/lazy";
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
import { startUpdateChecks } from "@/lib/update";
import { useMail } from "@/lib/store";
import { dragRegion } from "@/lib/platform";
import { t as tr } from "@/lib/i18n";
import { describeStored } from "@/lib/backendError";

/** Application verrouillée : le moteur refuserait toute lecture, les écouteurs d'événements s'abstiennent. */
const isLocked = () => useMail.getState().lock?.locked === true;

// Écrans rarement ouverts : chargés à la demande, hors du paquet initial.
const Onboarding = lazy(() => import("@/components/Onboarding").then((m) => ({ default: m.Onboarding })));
const Composer = lazy(() => loadComposer().then((m) => ({ default: m.Composer })));
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
        const locked = isLocked();
        if (!locked) void useMail.getState().reload({ silent: true }); // verrouillé, le moteur refuserait
        void notifyNewMail(e.payload, locked);
      }));
      // Message envoyé, aussitôt montré dans « Envoyés » (avant que le serveur n'ait la copie).
      unlisten.push(listen("sent-copy:shown", () => {
        if (isLocked()) return;
        const { view, reload, refreshAccounts } = useMail.getState();
        void refreshAccounts(); // « copies en attente » (barre latérale, réglages)
        const showsSent = view.kind !== "role" || view.role === "sent"; // les autres boîtes se relisent à la synchro
        if (showsSent) void reload({ silent: true });
      }));
      unlisten.push(listen<{ accountId: string; remaining: number }>("mail:backfill", (e) => useMail.getState().setBackfill(e.payload.accountId, e.payload.remaining)));
      unlisten.push(listen("mail:status", () => { if (!isLocked()) void useMail.getState().refreshAccounts(); }));
      unlisten.push(listen<number>("reminders:fired", (e) => {
        if (isLocked()) return;
        const { toast, reload } = useMail.getState();
        void reload({ silent: true });
        toast({ message: e.payload > 1 ? `${e.payload} conversations reviennent` : "Une conversation revient" });
      }));
      unlisten.push(listen<{ subject: string; error: string }>("sent-copy:failed", (e) => {
        useMail.getState().toast({ message: tr("composer:sentNoCopy", { subject: e.payload.subject || tr("list:noSubject"), error: describeStored(e.payload.error) }), tone: "error", duration: 8000 });
      }));
      unlisten.push(listen<string>("sent-copy:local", (e) => {
        useMail.getState().toast({ message: tr("composer:sentCopyLocal", { error: describeStored(e.payload) }), tone: "error", duration: 8000 });
      }));
      unlisten.push(listen<{ subject: string; error: string | null; outbox: boolean } | null>("scheduled:changed", (e) => {
        const { toast, refreshScheduled } = useMail.getState();
        void refreshScheduled();
        if (e.payload) toast(e.payload.error ? { message: tr("scheduled:sendFailedFor", { subject: e.payload.subject || tr("list:noSubject"), error: describeStored(e.payload.error) }), tone: "error", duration: 8000 } : { message: tr(e.payload.outbox ? "scheduled:outboxSent" : "scheduled:sentFor", { subject: e.payload.subject || tr("list:noSubject") }) });
      }));
    });
    return () => void unlisten.forEach((p) => p.then((fn) => fn()));
  }, []);

  useEffect(startUpdateChecks, []);
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
