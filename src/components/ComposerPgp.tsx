import { KeyRound, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Backend } from "@/lib/api";
import type { Draft } from "@/lib/compose";
import { t as tr } from "@/lib/i18n";
import { useMail } from "@/lib/store";
import { cn, sameAddr } from "@/lib/utils";
import { inputCls } from "./ui/primitives";

/** Adresse d'expéditeur du brouillon : l'alias choisi, sinon celle du compte. */
const senderAddress = (d: Draft, accounts: { id: string; email: string }[]) => d.fromEmail ?? accounts.find((a) => a.id === d.accountId)?.email ?? "";

/** Ce qui empêcherait de signer ou de chiffrer : pas de clé pour l'expéditeur, destinataires sans clé acceptée. */
function usePgpProblems(draft: Draft, backend: Backend, accounts: { id: string; email: string }[]) {
  const [noOwnKey, setNoOwnKey] = useState(false);
  const [missing, setMissing] = useState<string[]>([]);
  const sender = senderAddress(draft, accounts);
  const recipients = [...draft.to, ...draft.cc, ...draft.bcc].map((r) => r.address.toLowerCase());
  const key = JSON.stringify([draft.accountId, sender, recipients, !!draft.pgpEncrypt]);

  const active = !!(draft.pgpSign || draft.pgpEncrypt);
  useEffect(() => {
    if (!active) return;
    let live = true;
    void backend.pgpListSigningKeys().then(
      (l) => live && setNoOwnKey(!l.some((k) => k.accountId === draft.accountId && sameAddr(k.email, sender))),
      () => live && setNoOwnKey(true),
    );
    if (!draft.pgpEncrypt) setMissing([]);
    else {
      void Promise.all([...new Set(recipients)].map((a) => backend.pgpRecipientKey(a).then((fp) => (fp ? null : a), () => a))).then((r) => live && setMissing(r.filter((a): a is string => a !== null)));
    }
    return () => { live = false; };
    // `key` résume tout ce que l'effet lit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backend, key, active]);
  return { noOwnKey, missing };
}

/** Avertissements OpenPGP du composer et champ de la phrase secrète ; rien tant qu'on ne signe ni ne chiffre. */
export function PgpNotice({ draft, update }: { draft: Draft; update: (patch: Partial<Draft>) => void }) {
  const { backend, accounts } = useMail(useShallow((s) => ({ backend: s.backend, accounts: s.accounts })));
  const { noOwnKey, missing } = usePgpProblems(draft, backend, accounts);
  if (!draft.pgpSign && !draft.pgpEncrypt) return null;
  const warnings = [
    noOwnKey && tr("composer:pgp.noOwnKey", { email: senderAddress(draft, accounts) }),
    draft.pgpEncrypt && missing.length > 0 && tr("composer:pgp.missing", { count: missing.length, list: missing.join(", ") }),
  ].filter((w): w is string => !!w);
  return (
    <div className="flex shrink-0 flex-col gap-1.5 border-t border-border px-4 py-2 text-[12.5px]">
      {warnings.map((w) => (
        <p key={w} role="alert" className="flex items-start gap-1.5 text-danger"><TriangleAlert size={13} className="mt-0.5 shrink-0" />{w}</p>
      ))}
      {draft.pgpSign && draft.pgpAskPassphrase && (
        <label className="flex items-center gap-2">
          <KeyRound size={13} className="shrink-0 text-muted" />
          <span className="shrink-0 text-muted">{tr("composer:pgp.passphrase")}</span>
          <input type="password" autoFocus autoComplete="off" aria-label={tr("composer:pgp.passphrase")} value={draft.pgpPassphrase ?? ""} onChange={(e) => update({ pgpPassphrase: e.target.value })} className={cn(inputCls, "h-7 min-w-0 flex-1 px-2 text-[12.5px]")} />
        </label>
      )}
    </div>
  );
}
