import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Detected, OAuthProviderId, OAuthStatus, ServerConfig } from "@/lib/api";
import { isEmail } from "@/lib/compose";
import { OAUTH_INFO } from "@/lib/providers";
import { useMail } from "@/lib/store";
import { cn } from "@/lib/utils";
import { btnGhost, btnPrimary, inputCls, Spinner } from "./ui/primitives";

const SOURCES: Record<Detected["source"], string> = {
  builtin: "fournisseur connu",
  autoconfig: "configuration du fournisseur",
  ispdb: "base Mozilla (ISPDB)",
  mx: "fournisseur d'hébergement du domaine",
  guess: "supposés",
};
const tls = (host: string, port: number): ServerConfig => ({ host, port, security: "tls" });

/** Ajout d'un compte : les serveurs sont détectés à la saisie de l'adresse ; OAuth si le fournisseur le permet. */
export function AddAccountForm({ large = false }: { large?: boolean }) {
  const { backend, reload } = useMail(useShallow((s) => ({ backend: s.backend, reload: s.reload })));
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [client, setClient] = useState({ id: "", secret: "" });
  const [manual, setManual] = useState({ imap: "", smtp: "" });
  const [detected, setDetected] = useState<Detected | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [status, setStatus] = useState<OAuthStatus>({ google: false, microsoft: false });
  const [forcePassword, setForcePassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { void backend.oauthStatus().then(setStatus).catch(() => {}); }, [backend]);

  const addr = email.trim();
  useEffect(() => {
    setDetected(null);
    setForcePassword(false);
    if (!isEmail(addr)) return;
    let stale = false;
    setDetecting(true);
    const t = setTimeout(() => {
      backend.detectServers(addr)
        .then((d) => { if (!stale) { setDetected(d); setManual({ imap: d.imap.host, smtp: d.smtp.host }); } })
        .catch(() => {})
        .finally(() => { if (!stale) setDetecting(false); });
    }, 400);
    return () => { stale = true; clearTimeout(t); setDetecting(false); };
  }, [addr, backend]);

  const provider: OAuthProviderId | null = forcePassword ? null : detected?.oauth ?? null;
  const info = provider ? OAUTH_INFO[provider] : null;
  const configured = provider ? status[provider] : false;
  const guessed = detected?.source === "guess";
  const field = cn(inputCls, large && "h-10 text-[13.5px]");
  const wide = cn(btnPrimary, large && "h-10 w-full");

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      setEmail(""); setPassword(""); setClient({ id: "", secret: "" });
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const connect = (p: OAuthProviderId) => run(async () => {
    if (!status[p]) {
      await backend.setOAuthClient(p, client.id, client.secret || undefined);
      setStatus((s) => ({ ...s, [p]: true }));
    }
    await backend.addOAuthAccount(p, addr);
  });

  const addWithPassword = () => run(() => backend.addAccount({
    email: addr,
    password,
    // Hôtes corrigés à la main (détection par hypothèse uniquement) ; sinon le serveur redétecte.
    ...(guessed && detected ? { imap: tls(manual.imap.trim(), detected.imap.port), smtp: tls(manual.smtp.trim(), detected.smtp.port) } : {}),
  }));

  return (
    <form
      className={cn("space-y-2.5", !large && "max-w-[420px]")}
      onSubmit={(e) => {
        e.preventDefault();
        if (!detected) return;
        if (provider) void connect(provider);
        else void addWithPassword();
      }}
    >
      <input className={field} type="email" autoFocus={large} autoComplete="off" placeholder="Adresse e-mail" value={email} onChange={(e) => setEmail(e.target.value)} />

      {detecting && <p className="flex items-center gap-2 text-[12px] text-fg-muted"><Spinner /> Détection des serveurs…</p>}

      {detected && !detecting && (
        <p className="selectable text-[12px] text-fg-muted">
          {guessed
            ? "Serveurs introuvables automatiquement : vérifie-les ci-dessous."
            : `Serveurs détectés (${SOURCES[detected.source]}) : ${detected.imap.host}`}
        </p>
      )}

      {!detected || detecting ? null : provider && info ? (
        <>
          {!configured && (
            <div className="space-y-2 rounded-lg bg-black/[0.03] p-3 text-[12.5px] leading-relaxed text-fg-muted dark:bg-white/[0.05]">
              <p className="font-medium text-fg">Une seule fois : crée ton application {info.label}</p>
              <p>Sans double authentification : tu te connectes dans le navigateur, avec ton mot de passe habituel.</p>
              <ol className="selectable list-decimal space-y-1 pl-4">{info.steps.map((s) => <li key={s}>{s}</li>)}</ol>
              <input className={field} autoComplete="off" spellCheck={false} placeholder="ID client" value={client.id} onChange={(e) => setClient({ ...client, id: e.target.value })} />
              {info.needsSecret && <input className={field} type="password" autoComplete="off" placeholder="Code secret du client" value={client.secret} onChange={(e) => setClient({ ...client, secret: e.target.value })} />}
            </div>
          )}
          <button className={wide} disabled={busy || (!configured && !client.id.trim())}>
            {busy && <Spinner />}
            {configured ? `Se connecter avec ${info.label}` : `Enregistrer et se connecter avec ${info.label}`}
          </button>
          <div className="flex flex-wrap gap-2">
            <button type="button" className={btnGhost} onClick={() => setForcePassword(true)}>Mot de passe d'application</button>
            {configured && (
              <button type="button" className={btnGhost} disabled={busy} onClick={() => void backend.clearOAuthClient(provider).then(() => setStatus((s) => ({ ...s, [provider]: false })))}>
                Changer l'identifiant client
              </button>
            )}
          </div>
        </>
      ) : (
        <>
          {guessed && (
            <div className="grid grid-cols-2 gap-2">
              <input className={field} autoComplete="off" spellCheck={false} placeholder="Serveur IMAP" value={manual.imap} onChange={(e) => setManual({ ...manual, imap: e.target.value })} />
              <input className={field} autoComplete="off" spellCheck={false} placeholder="Serveur SMTP" value={manual.smtp} onChange={(e) => setManual({ ...manual, smtp: e.target.value })} />
            </div>
          )}
          <input className={field} type="password" autoComplete="off" placeholder="Mot de passe (ou mot de passe d'application)" value={password} onChange={(e) => setPassword(e.target.value)} />
          <button className={wide} disabled={busy || detecting || !detected || !password || (guessed && (!manual.imap.trim() || !manual.smtp.trim()))}>
            {busy && <Spinner />}
            {large ? "Se connecter" : "Ajouter"}
          </button>
        </>
      )}
      {error && <p className="selectable rounded-lg bg-danger/10 px-3 py-2 text-[12.5px] text-danger">{error}</p>}
    </form>
  );
}
