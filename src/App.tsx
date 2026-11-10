import { lazy, Suspense, useEffect } from "react";
import { useShallow } from "zustand/react/shallow";
import { Reader } from "@/components/Reader";
import { Sidebar } from "@/components/Sidebar";
import { ThreadList } from "@/components/ThreadList";
import { Toasts } from "@/components/Toasts";
import { TooltipProvider } from "@/components/ui/primitives";
import { useShortcuts } from "@/hooks/useShortcuts";
import { isTauri } from "@/lib/api";
import { useComposer } from "@/lib/composer";
import { useMail } from "@/lib/store";

// Écrans rarement ouverts : chargés à la demande, hors du paquet initial.
const Onboarding = lazy(() => import("@/components/Onboarding").then((m) => ({ default: m.Onboarding })));
const Composer = lazy(() => import("@/components/Composer").then((m) => ({ default: m.Composer })));
const Palette = lazy(() => import("@/components/Palette").then((m) => ({ default: m.Palette })));
const Settings = lazy(() => import("@/components/Settings").then((m) => ({ default: m.Settings })));
const ShortcutsHelp = lazy(() => import("@/components/Shortcuts").then((m) => ({ default: m.ShortcutsHelp })));

export default function App() {
  const { ready, needsOnboarding, density, paletteOpen, settingsOpen, helpOpen } = useMail(
    useShallow((s) => ({ ready: s.ready, needsOnboarding: s.needsOnboarding, density: s.density, paletteOpen: s.paletteOpen, settingsOpen: s.settingsTab !== null, helpOpen: s.helpOpen })),
  );
  const composing = useComposer((s) => s.draft !== null);
  useShortcuts();

  useEffect(() => {
    void useMail.getState().init();
    if (!isTauri()) return;
    // Événements du démon de synchro : la liste se met à jour sans geste de l'utilisateur.
    const unlisten: Promise<() => void>[] = [];
    import("@tauri-apps/api/event").then(({ listen }) => {
      unlisten.push(listen("mail:synced", () => void useMail.getState().reload({ silent: true })));
      unlisten.push(listen("mail:status", () => void useMail.getState().refreshAccounts()));
    });
    return () => void unlisten.forEach((p) => p.then((fn) => fn()));
  }, []);

  if (!ready) return <div className="h-full bg-bg" data-tauri-drag-region />;

  return (
    <TooltipProvider delayDuration={350}>
      {needsOnboarding ? (
        <Suspense fallback={null}><Onboarding /></Suspense>
      ) : (
        <div
          data-density={density}
          className="grid h-full grid-cols-[232px_minmax(340px,420px)_minmax(0,1fr)] max-[1100px]:grid-cols-[60px_minmax(320px,380px)_minmax(0,1fr)]"
        >
          <Sidebar />
          <ThreadList />
          <Reader />
        </div>
      )}
      <Suspense fallback={null}>
        {composing && <Composer />}
        {paletteOpen && <Palette />}
        {settingsOpen && <Settings />}
        {helpOpen && <ShortcutsHelp />}
      </Suspense>
      <Toasts />
    </TooltipProvider>
  );
}
