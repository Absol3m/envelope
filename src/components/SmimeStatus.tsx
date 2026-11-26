import { useEffect, useMemo, useRef, useState } from "react";
import type { DetailMessage, SmimeInspection, SmimeState } from "@/lib/api";
import { t as tr } from "@/lib/i18n";
import { useMail } from "@/lib/store";
import { Row } from "./PgpStatus";

/** Lecture S/MIME d'un message ouvert : le moteur reconnaît la forme (`smime_inspect` rend `null` sinon), une seule fois. */
export function useSmime(m: DetailMessage, enabled: boolean) {
  const backend = useMail((s) => s.backend);
  const [result, setResult] = useState<SmimeInspection | null>(null);
  const loaded = useRef(false);
  useEffect(() => {
    if (!enabled || loaded.current || m.oversize) return;
    loaded.current = true;
    void backend.smimeInspect(m.id).then(setResult, () => setResult(null));
  }, [enabled, m.oversize, m.id, backend]);
  // Contenu d'une signature opaque : remplace le corps stocké pour l'affichage.
  const shown = useMemo(
    () => (result && (result.bodyText !== null || result.bodyHtml !== null) ? { ...m, bodyText: result.bodyText, bodyHtml: result.bodyHtml } : m),
    [m, result],
  );
  return { result, shown };
}

const KEYS: Record<SmimeState, { key: string; tone: "success" | "danger" | "neutral" }> = {
  valid: { key: "valid", tone: "success" },
  invalid: { key: "invalid", tone: "danger" },
  untrusted: { key: "untrusted", tone: "danger" },
  address_mismatch: { key: "addressMismatch", tone: "danger" },
  no_address: { key: "noAddress", tone: "neutral" },
  time_mismatch: { key: "timeMismatch", tone: "neutral" },
  unsupported: { key: "unsupported", tone: "neutral" },
};

/** Bandeau S/MIME d'un message : verdict de signature (le déchiffrement n'est pas encore pris en charge). */
export function SmimeBanner({ smime }: { smime: ReturnType<typeof useSmime> }) {
  const { result } = smime;
  if (!result) return null;
  const sig = result.signature;
  const shown = result.encrypted ? { key: "encrypted", tone: "neutral" as const } : sig ? KEYS[sig.state] : null;
  if (!shown) return null;
  return <Row tone={shown.tone}>{tr(`reader:smime.${shown.key}`, { signer: sig?.signer ?? "" })}</Row>;
}
