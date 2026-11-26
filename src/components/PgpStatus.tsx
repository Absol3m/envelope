import { KeyRound, Lock, LockOpen, ShieldAlert, ShieldCheck, ShieldQuestion } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { DetailMessage, PgpInspection } from "@/lib/api";
import { t as tr } from "@/lib/i18n";
import { useMail } from "@/lib/store";
import { cn } from "@/lib/utils";
import { btnPrimary, inputCls } from "./ui/primitives";

/** Lecture PGP d'un message ouvert : c'est le moteur qui reconnaît la forme PGP (`pgp_inspect` rend `null` sinon), une
 *  seule fois par message ; `unlock` la relance avec la phrase secrète de la clé privée. */
export function usePgp(m: DetailMessage, enabled: boolean) {
  const backend = useMail((s) => s.backend);
  const [result, setResult] = useState<PgpInspection | null>(null);
  const [busy, setBusy] = useState(false);
  const [tried, setTried] = useState(false);
  const asked = useRef(0); // jeton : une réponse tardive d'une demande plus ancienne est ignorée
  const loaded = useRef(false);
  const run = useCallback(async (passphrase?: string) => {
    const ticket = ++asked.current;
    setBusy(true);
    if (passphrase) setTried(true);
    try {
      const r = await backend.pgpInspect(m.id, passphrase);
      if (ticket === asked.current) setResult(r);
    } catch { if (ticket === asked.current) setResult(null); } finally { if (ticket === asked.current) setBusy(false); }
  }, [backend, m.id]);
  useEffect(() => {
    if (!enabled || loaded.current || m.oversize) return;
    loaded.current = true;
    void run();
  }, [enabled, m.oversize, run]);
  // Contenu déchiffré (ou texte d'un message signé en clair) : remplace le corps stocké pour l'affichage.
  const shown = useMemo(
    () => (result && (result.bodyText !== null || result.bodyHtml !== null) ? { ...m, bodyText: result.bodyText, bodyHtml: result.bodyHtml } : m),
    [m, result],
  );
  return { result, busy, tried, unlock: run, shown };
}

const TONES = {
  success: { icon: ShieldCheck, cls: "bg-success/10", iconCls: "text-success" },
  danger: { icon: ShieldAlert, cls: "bg-danger/10", iconCls: "text-danger" },
  neutral: { icon: ShieldQuestion, cls: "bg-bg-subtle", iconCls: "text-muted" },
  info: { icon: LockOpen, cls: "bg-bg-subtle", iconCls: "text-success" },
  locked: { icon: Lock, cls: "bg-warn/10", iconCls: "text-warn" },
} as const;

export function Row({ tone, children }: { tone: keyof typeof TONES; children: ReactNode }) {
  const { icon: Icon, cls, iconCls } = TONES[tone];
  return (
    <div className={cn("anim-fade-in mt-3 flex items-start gap-2.5 rounded-lg px-3 py-2 text-[12.5px] text-fg", cls)}>
      <Icon size={15} className={cn("mt-px shrink-0", iconCls)} />
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}

function PassphraseForm({ busy, wrong, onSubmit }: { busy: boolean; wrong: boolean; onSubmit(passphrase: string): void }) {
  const [value, setValue] = useState("");
  return (
    <form className="flex flex-col gap-2" onSubmit={(e) => { e.preventDefault(); if (value) onSubmit(value); }}>
      <span>{wrong ? tr("reader:pgp.passphraseWrong") : tr("reader:pgp.passphrase")}</span>
      <div className="flex gap-2">
        <input type="password" autoComplete="off" aria-label={tr("reader:pgp.passphraseLabel")} value={value} onChange={(e) => setValue(e.target.value)} className={cn(inputCls, "max-w-[280px]")} />
        <button type="submit" disabled={busy || !value} className={cn(btnPrimary, "gap-1.5")}><KeyRound size={13} />{tr("reader:pgp.decrypt")}</button>
      </div>
    </form>
  );
}

/** Bandeau OpenPGP d'un message : verdict de signature, état du déchiffrement. */
export function PgpBanner({ pgp }: { pgp: ReturnType<typeof usePgp> }) {
  const { result, busy, tried, unlock } = pgp;
  if (!result) return null;
  const sig = result.signature;
  const signer = sig?.signer ?? sig?.fingerprint ?? "";
  return (
    <>
      {result.encrypted && result.error === "passphrase" && (
        <Row tone="locked"><PassphraseForm busy={busy} wrong={tried} onSubmit={(p) => void unlock(p)} /></Row>
      )}
      {result.encrypted && result.error === "no_key" && <Row tone="neutral">{tr("reader:pgp.noKey")}</Row>}
      {result.encrypted && result.error === "failed" && <Row tone="danger">{tr("reader:pgp.failed")}</Row>}
      {result.encrypted && !result.error && <Row tone="info">{tr("reader:pgp.decrypted")}</Row>}
      {sig?.state === "valid" && <Row tone="success">{tr("reader:pgp.valid", { signer })}</Row>}
      {sig?.state === "invalid" && <Row tone="danger"><strong className="font-semibold">{tr("reader:pgp.invalid", { signer })}</strong></Row>}
      {sig?.state === "unknown_key" && <Row tone="neutral">{tr("reader:pgp.unknownKey")}</Row>}
    </>
  );
}
