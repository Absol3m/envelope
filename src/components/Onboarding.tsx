import { Lock, Sparkles } from "lucide-react";
import { useMail } from "@/lib/store";
import { dragRegion } from "@/lib/platform";
import { AddAccountForm } from "./AddAccountForm";

/** Premier lancement : un seul écran, un seul choix. */
export function Onboarding() {
  const enterDemo = useMail((s) => s.enterDemo);

  return (
    <div className="relative flex h-full items-center justify-center overflow-hidden bg-bg" {...dragRegion}>
      <div aria-hidden className="pointer-events-none absolute -top-40 left-1/2 h-[520px] w-[820px] -translate-x-1/2 rounded-full bg-gradient-to-br from-accent/25 via-[#8b5cf6]/15 to-transparent blur-3xl" />
      <div className="anim-fade-up relative w-[min(420px,90vw)]">
        <div className="mb-7 text-center">
          <img src="/icon.svg" alt="" className="mx-auto -mb-1 -mt-4 h-24 w-24" draggable={false} />
          <h1 className="text-[26px] font-semibold tracking-tight">Bienvenue dans Envelope</h1>
          <p className="mt-1.5 text-[14px] text-muted">Le client mail rapide, privé et prêt pour vos agents IA.</p>
        </div>

        <div className="space-y-3 rounded-2xl bg-elevated p-5 shadow-pop">
          <AddAccountForm large />
          <p className="flex items-start gap-2 pt-1 text-[12px] text-muted"><Lock size={13} className="mt-px shrink-0" />Votre mot de passe est stocké dans le trousseau du système. Vos e-mails restent sur cet ordinateur.</p>
        </div>

        <button onClick={() => void enterDemo()} className="press mx-auto mt-5 flex items-center gap-2 rounded-full px-4 py-2 text-[13px] font-medium text-muted hover:bg-hover hover:text-fg">
          <Sparkles size={15} className="text-accent" /> Explorer avec des données de démo
        </button>
      </div>
    </div>
  );
}
