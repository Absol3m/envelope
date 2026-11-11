import * as Dialog from "@radix-ui/react-dialog";
import { Bot, Check, Copy, Keyboard, Monitor, Moon, Palette, RefreshCw, ShieldCheck, Sun, Trash2, UserRound, X } from "lucide-react";
import { useEffect, useState, type ComponentType, type ReactNode } from "react";
import { type McpInfo } from "@/lib/api";
import { relativeTime } from "@/lib/format";
import { customName, FREE_FONTS, type FontPref } from "@/lib/font";
import { notificationsEnabled, sendNative, setNotificationsEnabled } from "@/lib/notify";
import { useShallow } from "zustand/react/shallow";
import { useMail, type SettingsTab } from "@/lib/store";
import type { ColorsPref, ThemePref } from "@/lib/theme";
import { cn } from "@/lib/utils";
import { ShortcutList } from "./Shortcuts";
import { AddAccountForm } from "./AddAccountForm";
import { SETTINGS_LABEL } from "@/lib/platform";
import { Avatar, btnPrimary, IconButton, inputCls } from "./ui/primitives";

const TABS: { id: SettingsTab; label: string; icon: ComponentType<{ size?: number }> }[] = [
  { id: "accounts", label: "Comptes", icon: UserRound },
  { id: "agent", label: "Agent IA (MCP)", icon: Bot },
  { id: "appearance", label: "Apparence", icon: Palette },
  { id: "shortcuts", label: "Raccourcis", icon: Keyboard },
];


function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section className="mb-7">
      <h3 className="text-[13px] font-semibold">{title}</h3>
      {hint && <p className="mt-0.5 text-[12.5px] text-muted">{hint}</p>}
      <div className="mt-3">{children}</div>
    </section>
  );
}

function Segmented<T extends string>({ value, onChange, options }: { value: T; onChange(v: T): void; options: { value: T; label: string; icon?: ComponentType<{ size?: number }> }[] }) {
  return (
    <div role="radiogroup" className="inline-flex rounded-lg bg-hover p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={cn("press flex h-8 items-center gap-1.5 rounded-md px-3 text-[12.5px] font-medium", value === o.value ? "bg-bg shadow-card" : "text-muted hover:text-fg")}
        >
          {o.icon && <o.icon size={14} />}
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Switch({ checked, onChange, label }: { checked: boolean; onChange(v: boolean): void; label: string }) {
  return (
    <button role="switch" aria-checked={checked} aria-label={label} onClick={() => onChange(!checked)} className={cn("relative h-[22px] w-[38px] shrink-0 rounded-full transition-colors", checked ? "bg-accent" : "bg-active")}>
      <span className={cn("absolute top-[2px] h-[18px] w-[18px] rounded-full bg-white shadow transition-transform", checked ? "translate-x-[18px]" : "translate-x-[2px]")} />
    </button>
  );
}

function CopyButton({ text, label = "Copier" }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      onClick={() => { void navigator.clipboard?.writeText(text); setDone(true); setTimeout(() => setDone(false), 1500); }}
      className="press inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] font-medium text-muted hover:bg-active hover:text-fg"
    >
      {done ? <Check size={13} className="text-success" /> : <Copy size={13} />}
      {done ? "Copié" : label}
    </button>
  );
}

function Code({ title, text }: { title: string; text: string }) {
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-bg-subtle">
      <div className="flex items-center border-b border-border px-3 py-1">
        <span className="flex-1 text-[12px] font-medium text-muted">{title}</span>
        <CopyButton text={text} />
      </div>
      <pre className="selectable overflow-x-auto p-3 font-mono text-[12px] leading-relaxed">{text}</pre>
    </div>
  );
}

// ───────────── Comptes ─────────────

/** Signature d'un compte : enregistrée à la perte du focus, si elle a changé. */
function SignatureField({ id, email, value }: { id: string; email: string; value: string }) {
  const { backend, refreshAccounts, toast } = useMail(useShallow((s) => ({ backend: s.backend, refreshAccounts: s.refreshAccounts, toast: s.toast })));
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const save = async () => {
    if (text.trimEnd() === value) return;
    try {
      await backend.setAccountSignature(id, text);
      await refreshAccounts();
    } catch (e) {
      toast({ message: `Signature non enregistrée — ${String(e)}`, tone: "error" });
    }
  };
  return (
    <label className="block">
      <span className="mb-1 block text-[12px] text-muted">{email}</span>
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => void save()}
        rows={3}
        maxLength={2000}
        placeholder="Aucune signature"
        className={cn(inputCls, "h-auto resize-y py-2")}
      />
    </label>
  );
}

function AccountsPanel() {
  const { accounts, backend, reload, toast, init } = useMail(useShallow((s) => ({ accounts: s.accounts, backend: s.backend, reload: s.reload, toast: s.toast, init: s.init })));
  const [removing, setRemoving] = useState<string | null>(null);

  const remove = async (id: string, email: string) => {
    if (!confirm(`Supprimer ${email} d'Envelope ? Les e-mails restent sur le serveur.`)) return;
    setRemoving(id);
    try {
      await backend.removeAccount(id);
      await reload();
    } catch (e) {
      toast({ message: `Suppression impossible — ${String(e)}`, tone: "error" });
    } finally {
      setRemoving(null);
    }
  };

  return (
    <>
      {backend.demo && (
        <div className="mb-6 flex items-center gap-3 rounded-xl bg-accent-soft px-4 py-3 text-[13px]">
          <span className="flex-1">Vous explorez la boîte de démonstration. Connectez un vrai compte pour utiliser Envelope.</span>
          <button className={btnPrimary} onClick={() => void init()}>Quitter la démo</button>
        </div>
      )}
      <Section title="Comptes connectés">
        <ul className="divide-y divide-border rounded-xl border border-border">
          {accounts.map((a) => (
            <li key={a.id} className="flex items-center gap-3 px-3.5 py-3">
              <Avatar name={a.displayName} address={a.email} size={34} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-[13px] font-medium">{a.displayName ?? a.email}</p>
                <p className="truncate text-[12px] text-muted">{a.email} · {a.authKind === "oauth2" ? "OAuth2" : "mot de passe"}</p>
                <p className={cn("text-[11.5px]", a.syncStatus === "error" ? "text-danger" : "text-faint")}>
                  {a.syncStatus === "error" ? a.syncError ?? "Erreur de synchronisation" : a.syncStatus === "syncing" ? "Synchronisation…" : `Synchronisé ${relativeTime(a.lastSyncAt)}`}
                </p>
              </div>
              <IconButton label="Synchroniser" onClick={() => void backend.syncNow(a.id).then(() => toast({ message: "Synchronisation lancée" }))}><RefreshCw size={15} /></IconButton>
              <IconButton label="Supprimer le compte" disabled={backend.demo || removing !== null} onClick={() => void remove(a.id, a.email)}><Trash2 size={15} /></IconButton>
            </li>
          ))}
          {accounts.length === 0 && <li className="px-3.5 py-6 text-center text-[13px] text-muted">Aucun compte pour l'instant.</li>}
        </ul>
      </Section>

      {accounts.length > 0 && (
        <Section title="Signatures" hint="Ajoutée aux nouveaux messages et aux réponses, précédée de « -- ». Texte brut.">
          <div className="space-y-3">
            {accounts.map((a) => <SignatureField key={a.id} id={a.id} email={a.email} value={a.signature} />)}
          </div>
        </Section>
      )}

      {!backend.demo && (
        <Section title="Ajouter un compte" hint="Gmail, iCloud, Yahoo… : utilisez un mot de passe d'application, pas votre mot de passe habituel. Il est stocké dans le trousseau du système.">
          <AddAccountForm />
        </Section>
      )}
    </>
  );
}

// ───────────── Agent IA (MCP) ─────────────

const TOOLS: [string, string][] = [
  ["list_accounts", "Comptes configurés"],
  ["list_folders", "Dossiers et compteurs de non lus"],
  ["search_emails", "Recherche plein texte, extraits courts"],
  ["get_thread", "Lire une conversation (corps bornés)"],
  ["summarize_thread", "Transcription condensée, sans citations"],
];

function AgentPanel() {
  const backend = useMail((s) => s.backend);
  const [info, setInfo] = useState<McpInfo | null>(null);
  useEffect(() => { backend.mcpInfo().then(setInfo, () => undefined); }, [backend]);
  const bin = info?.binaryPath ?? "/chemin/vers/envelope-mcp";
  const json = (extra = "") => JSON.stringify({ mcpServers: { envelope: { command: bin, ...(extra ? { args: [extra] } : {}) } } }, null, 2);

  return (
    <>
      <div className="mb-7 flex items-center gap-4 rounded-2xl bg-gradient-to-br from-accent/12 to-[#8b5cf6]/10 p-5">
        <span className="inline-flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-bg text-accent shadow-card"><Bot size={24} strokeWidth={1.6} /></span>
        <div className="min-w-0 flex-1">
          <p className="text-[15px] font-semibold tracking-tight">Laissez un agent IA lire votre boîte</p>
          <p className="mt-0.5 text-[12.5px] text-muted">Claude, Cursor ou tout client MCP peut chercher et résumer vos e-mails — en lecture seule, en local.</p>
        </div>
        <span className={cn("shrink-0 rounded-full px-2.5 py-1 text-[11.5px] font-semibold", info?.binaryPath ? "bg-success/12 text-success" : "bg-warn/15 text-warn")}>
          {info?.binaryPath ? "Serveur prêt" : "Binaire introuvable"}
        </span>
      </div>

      <Section title="Connecter un agent" hint={info?.binaryPath ? undefined : "Compilez d'abord le serveur : cargo build --release -p envelope-mcp (dans src-tauri/)."}>
        <div className="space-y-3">
          <Code title="Claude Code" text={`claude mcp add envelope -- ${bin}`} />
          <Code title="Claude Desktop / Cursor (mcpServers)" text={json()} />
          {info && <p className="selectable text-[12px] text-muted">Base lue : <span className="font-mono">{info.dbPath}</span> (option <span className="font-mono">--db</span> ou variable <span className="font-mono">ENVELOPE_DB</span> pour en changer).</p>}
        </div>
      </Section>

      <Section title="Ce que l'agent peut faire">
        <ul className="divide-y divide-border rounded-xl border border-border">
          {TOOLS.map(([name, desc]) => (
            <li key={name} className="flex items-center gap-3 px-3.5 py-2.5">
              <code className="rounded-md bg-hover px-1.5 py-0.5 font-mono text-[12px]">{name}</code>
              <span className="flex-1 text-[12.5px] text-muted">{desc}</span>
              <span className="rounded-full bg-success/12 px-2 py-0.5 text-[11px] font-semibold text-success">Lecture seule</span>
            </li>
          ))}
        </ul>
      </Section>

      <Section title="Garanties">
        <ul className="space-y-2 text-[13px]">
          {[
            "Aucune écriture : l'agent ne peut ni archiver, ni envoyer, ni supprimer.",
            "Le contenu des e-mails est signalé comme non fiable : un message piégé ne peut pas donner d'ordres à l'agent.",
            "Les pièces jointes ne sont jamais exposées — seulement leur nom, leur type et leur taille.",
            "Tout reste sur votre machine : le serveur parle à l'agent par stdio, sans réseau.",
          ].map((t) => (
            <li key={t} className="flex gap-2.5"><ShieldCheck size={16} className="mt-px shrink-0 text-success" />{t}</li>
          ))}
        </ul>
      </Section>
    </>
  );
}

// ───────────── Apparence ─────────────

/** Police de l'interface : celle du système par défaut, des polices libres, ou une police installée. */
function FontSection() {
  const { font, setFont } = useMail(useShallow((s) => ({ font: s.font, setFont: s.setFont })));
  const isCustom = font.startsWith("custom:");
  const [typed, setTyped] = useState(isCustom ? font.slice(7) : "");
  const valid = customName(typed) !== null;
  const selected = isCustom ? "custom" : font;
  const note = FREE_FONTS.find((f) => f.id === font)?.note;
  return (
    <Section title="Police" hint="Par défaut, la police de votre système. Les polices libres (licence OFL) ne sont téléchargées que si vous les choisissez ; vous pouvez aussi saisir le nom d'une police installée sur cet ordinateur.">
      <div className="flex max-w-[420px] flex-col gap-2.5">
        <select
          aria-label="Police de l'interface"
          value={selected}
          onChange={(e) => {
            const v = e.target.value;
            if (v === "custom") setFont((customName(typed) ? `custom:${customName(typed)}` : "system") as FontPref);
            else setFont(v as FontPref);
            if (v === "custom" && !customName(typed)) setTyped("");
          }}
          className={cn(inputCls, "px-2")}
        >
          <option value="system">Police du système (par défaut)</option>
          {FREE_FONTS.map((f) => <option key={f.id} value={f.id}>{f.label}</option>)}
          <option value="custom">Une police installée…</option>
        </select>
        {(selected === "custom" || isCustom) && (
          <div className="flex gap-2">
            <input
              aria-label="Nom de la police installée"
              className={cn(inputCls, "min-w-0 flex-1")}
              autoComplete="off"
              spellCheck={false}
              placeholder="Nom exact, ex. Helvetica Neue"
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && valid) setFont(`custom:${customName(typed)!}`); }}
            />
            <button className={btnPrimary} disabled={!valid} onClick={() => setFont(`custom:${customName(typed)!}`)}>Appliquer</button>
          </div>
        )}
        {note && <p className="text-[12px] text-muted">{note}</p>}
        <p className="rounded-lg bg-bg-subtle px-3 py-2.5 text-[13px] leading-relaxed">
          Aa Bb Cc — Le vif zéphyr jubile sur les kumquats du clown gracieux. 0123456789
        </p>
      </div>
    </Section>
  );
}

function NotificationsSection() {
  const [on, setOn] = useState(notificationsEnabled);
  const toast = useMail((s) => s.toast);
  const test = async () => {
    try {
      await sendNative({ title: "Envelope", body: "Les notifications fonctionnent." });
    } catch (e) {
      toast({ message: `Notification impossible — ${String(e)}`, tone: "error", duration: 6000 });
    }
  };
  return (
    <Section title="Notifications" hint="Une notification du système à l'arrivée d'un message dans la boîte de réception, quand Envelope n'est pas au premier plan.">
      <label className="flex max-w-[420px] items-center gap-3 text-[13px]">
        <span className="flex-1">Notifier les nouveaux messages</span>
        <Switch checked={on} onChange={(v) => { setOn(v); setNotificationsEnabled(v); }} label="Notifier les nouveaux messages" />
      </label>
      <button className="press mt-3 h-8 rounded-lg border border-border bg-bg px-3 text-[12.5px] font-medium hover:bg-hover" onClick={() => void test()}>Envoyer une notification de test</button>
    </Section>
  );
}

function AppearancePanel() {
  const s = useMail();
  return (
    <>
      <Section title="Thème">
        <Segmented<ThemePref> value={s.theme} onChange={s.setTheme} options={[{ value: "system", label: "Système", icon: Monitor }, { value: "light", label: "Clair", icon: Sun }, { value: "dark", label: "Sombre", icon: Moon }]} />
      </Section>
      <Section title="Couleurs" hint="« Système » reprend les couleurs et la couleur d'accent de votre système (et de votre bureau sous Linux).">
        <Segmented<ColorsPref> value={s.colors} onChange={s.setColors} options={[{ value: "system", label: "Système", icon: Monitor }, { value: "envelope", label: "Envelope", icon: Palette }]} />
      </Section>
      <Section title="Densité de la liste">
        <Segmented value={s.density} onChange={s.setDensity} options={[{ value: "comfortable", label: "Confortable" }, { value: "compact", label: "Compacte" }]} />
      </Section>
      <FontSection />
      <NotificationsSection />
      <Section title="Aperçus">
        <label className="flex max-w-[420px] items-center gap-3 text-[13px]">
          <span className="flex-1">Afficher un extrait du message sous l'objet</span>
          <Switch checked={s.previews} onChange={s.setPreviews} label="Afficher les aperçus" />
        </label>
      </Section>
    </>
  );
}

// ───────────── Fenêtre ─────────────

export function Settings() {
  const { settingsTab, openSettings } = useMail(useShallow((s) => ({ settingsTab: s.settingsTab, openSettings: s.openSettings })));
  return (
    <Dialog.Root open={settingsTab !== null} onOpenChange={(o) => !o && openSettings(null)}>
      <Dialog.Portal>
        <Dialog.Overlay className="anim-fade-in fixed inset-0 z-[90] bg-black/35 backdrop-blur-[3px]" />
        <Dialog.Content
          aria-describedby={undefined}
          onOpenAutoFocus={(e) => e.preventDefault()}
          className="fixed left-1/2 top-1/2 z-[91] flex h-[min(640px,88vh)] w-[min(900px,94vw)] overflow-hidden rounded-2xl bg-elevated shadow-pop"
          style={{ animation: "dialog-in 0.22s cubic-bezier(0.16,1,0.3,1) both" }}
        >
          <nav className="w-[196px] shrink-0 border-r border-border bg-sidebar p-3" aria-label={SETTINGS_LABEL}>
            <Dialog.Title className="px-2.5 pb-3 pt-1 text-[15px] font-semibold tracking-tight">{SETTINGS_LABEL}</Dialog.Title>
            {TABS.map((t) => (
              <button key={t.id} onClick={() => openSettings(t.id)} aria-current={settingsTab === t.id} className={cn("press flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-[13px] font-medium", settingsTab === t.id ? "bg-active text-fg" : "text-muted hover:bg-hover hover:text-fg")}>
                <t.icon size={15} /> {t.label}
              </button>
            ))}
          </nav>
          <div className="relative min-w-0 flex-1 overflow-y-auto p-7">
            <Dialog.Close asChild><IconButton label="Fermer" className="absolute right-3 top-3"><X size={16} /></IconButton></Dialog.Close>
            <h2 className="mb-5 text-[18px] font-semibold tracking-tight">{TABS.find((t) => t.id === settingsTab)?.label}</h2>
            {settingsTab === "accounts" && <AccountsPanel />}
            {settingsTab === "agent" && <AgentPanel />}
            {settingsTab === "appearance" && <AppearancePanel />}
            {settingsTab === "shortcuts" && <ShortcutList />}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
