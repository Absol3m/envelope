import * as Dialog from "@radix-ui/react-dialog";
import { Bot, Check, Download, Filter, Copy, Info, Keyboard, Monitor, Moon, Palette, RefreshCw, ShieldCheck, Sun, Trash2, UserRound, X } from "lucide-react";
import { useEffect, useState, type ComponentType, type ReactNode } from "react";
import { Trans, useTranslation } from "react-i18next";
import { LANGUAGES, parseLangPref, setLanguage, type LangPref } from "@/lib/i18n";
import { pref } from "@/lib/pref";
import { isTauri, type Account, type LockMethod, type McpInfo } from "@/lib/api";
import { senderLabel } from "@/lib/compose";
import { relativeTime } from "@/lib/format";
import { serverDraftsEnabled, setServerDrafts } from "@/lib/composer";
import { customName, FREE_FONTS, type FontPref } from "@/lib/font";
import { notificationsEnabled, sendNative, setNotificationsEnabled } from "@/lib/notify";
import { autoUpdateEnabled, checkForUpdate, installUpdate, setAutoUpdateEnabled, useUpdate } from "@/lib/update";
import { useShallow } from "zustand/react/shallow";
import { useMail, type SettingsTab } from "@/lib/store";
import type { ColorsPref, ThemePref } from "@/lib/theme";
import { cn, errText } from "@/lib/utils";
import { ShortcutList } from "./Shortcuts";
import { AddAccountForm } from "./AddAccountForm";
import { RulesPanel } from "./RulesPanel";
import { isMac, isWindows, settingsLabel } from "@/lib/platform";
import { Avatar, btnGhost, btnPrimary, DialogFrame, IconButton, inputCls, Spinner } from "./ui/primitives";
import { t as tr } from "@/lib/i18n";
import { describeStored } from "@/lib/backendError";

const tab = (id: SettingsTab, icon: ComponentType<{ size?: number }>) => ({ id, icon, get label() { return tr(`settings:tabs.${id}`); } });
const TABS = [tab("accounts", UserRound), tab("rules", Filter), tab("security", ShieldCheck), tab("agent", Bot), tab("appearance", Palette), tab("shortcuts", Keyboard), tab("about", Info)];


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

function CopyButton({ text, label = tr("settings:copy") }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      onClick={() => { void navigator.clipboard?.writeText(text); setDone(true); setTimeout(() => setDone(false), 1500); }}
      className="press inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] font-medium text-muted hover:bg-active hover:text-fg"
    >
      {done ? <Check size={13} className="text-success" /> : <Copy size={13} />}
      {done ? tr("settings:copied") : label}
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

/** Copie des messages envoyés dans « Envoyés », par compte (Gmail la range lui-même : réglage sans effet, grisé). */
function CopySentField({ account }: { account: Account }) {
  const { backend, refreshAccounts, toast } = useMail(useShallow((s) => ({ backend: s.backend, refreshAccounts: s.refreshAccounts, toast: s.toast })));
  const byServer = account.provider === "gmail";
  const change = (on: boolean) => backend.setAccountCopySent(account.id, on).then(refreshAccounts, (e) => toast({ message: String(e), tone: "error" }));
  return (
    <label className={cn("flex max-w-[420px] items-center gap-3 text-[13px]", byServer && "opacity-60")}>
      <span className="flex-1">{account.email}{byServer && <span className="block text-[12px] text-muted">{tr("settings:copySent.byServer")}</span>}</span>
      <Switch checked={byServer || account.copySent} onChange={(v) => void change(v)} label={tr("settings:copySent.label", { email: account.email })} />
    </label>
  );
}

/** Signature d'un compte ou d'un alias : enregistrée à la perte du focus, si elle a changé. */
function SignatureField({ label, value, onSave, placeholder }: { label: string; value: string; onSave: (text: string) => Promise<void>; placeholder?: string }) {
  const { refreshAccounts, toast } = useMail(useShallow((s) => ({ refreshAccounts: s.refreshAccounts, toast: s.toast })));
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const save = async () => {
    if (text.trimEnd() === value) return;
    try {
      await onSave(text);
      await refreshAccounts();
    } catch (e) {
      toast({ message: tr("settings:signatureFailed", { error: String(e) }), tone: "error" });
    }
  };
  return (
    <label className="block">
      <span className="mb-1 block text-[12px] text-muted">{label}</span>
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => void save()}
        rows={3}
        maxLength={2000}
        placeholder={placeholder ?? tr("settings:noSignature")}
        className={cn(inputCls, "h-auto resize-y py-2")}
      />
    </label>
  );
}

/** Signature du compte, puis celle de chacun de ses alias (vide : celle du compte s'applique). */
function AccountSignatures({ account }: { account: Account }) {
  const backend = useMail((s) => s.backend);
  return (
    <>
      <SignatureField label={account.email} value={account.signature} onSave={(text) => backend.setAccountSignature(account.id, text)} />
      {account.identities.map((i) => (
        <SignatureField key={i.id} label={i.email} value={i.signature} onSave={(text) => backend.setIdentitySignature(account.id, i.id, text)} placeholder={tr("settings:aliasSignature")} />
      ))}
    </>
  );
}

/** Alias d'envoi d'un compte : liste, ajout (adresse + nom affiché) et retrait. */
function IdentitiesField({ account }: { account: Account }) {
  const { backend, refreshAccounts, toast } = useMail(useShallow((s) => ({ backend: s.backend, refreshAccounts: s.refreshAccounts, toast: s.toast })));
  const [email, setEmail] = useState("");
  const [name, setName] = useState("");
  const run = (job: Promise<void>, failure: string) => job.then(refreshAccounts).catch((e) => toast({ message: `${failure} — ${String(e)}`, tone: "error" }));
  return (
    <div>
      <span className="mb-1 block text-[12px] text-muted">{account.email}</span>
      <ul className="mb-2 space-y-1">
        {account.identities.map((i) => (
          <li key={i.id} className="flex items-center gap-2 rounded-lg bg-hover px-2.5 py-1 text-[13px]">
            <span className="min-w-0 flex-1 truncate">{senderLabel(i.name, i.email)}</span>
            <button type="button" aria-label={tr("composer:remove", { name: i.email })} onClick={() => void run(backend.removeIdentity(account.id, i.id), tr("errors:delete"))} className="press rounded p-0.5 text-muted hover:bg-active hover:text-danger"><X size={13} /></button>
          </li>
        ))}
      </ul>
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (!email.trim()) return;
          void run(backend.addIdentity(account.id, name, email), tr("settings:addFailed")).then(() => { setEmail(""); setName(""); });
        }}
      >
        <input aria-label={tr("settings:aliasAddress")} type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder={tr("settings:aliasPlaceholder")} className={cn(inputCls, "min-w-0 flex-[2]")} />
        <input aria-label={tr("settings:displayName")} value={name} onChange={(e) => setName(e.target.value)} maxLength={100} placeholder={tr("settings:displayNameOptional")} className={cn(inputCls, "min-w-0 flex-1")} />
        <button type="submit" className={btnPrimary}>{tr("settings:add")}</button>
      </form>
    </div>
  );
}

function AccountsPanel() {
  const { accounts, backend, reload, toast, init } = useMail(useShallow((s) => ({ accounts: s.accounts, backend: s.backend, reload: s.reload, toast: s.toast, init: s.init })));
  const [removing, setRemoving] = useState<string | null>(null);

  const remove = async (id: string, email: string) => {
    if (!confirm(tr("settings:removeConfirm", { email }))) return;
    setRemoving(id);
    try {
      await backend.removeAccount(id);
      await reload();
    } catch (e) {
      toast({ message: tr("settings:removeFailed", { error: String(e) }), tone: "error" });
    } finally {
      setRemoving(null);
    }
  };

  return (
    <>
      {backend.demo && (
        <div className="mb-6 flex items-center gap-3 rounded-xl bg-accent-soft px-4 py-3 text-[13px]">
          <span className="flex-1">{tr("settings:demoNotice")}</span>
          <button className={btnPrimary} onClick={() => void init()}>{tr("settings:leaveDemo")}</button>
        </div>
      )}
      <Section title={tr("settings:connectedAccounts")}>
        <ul className="divide-y divide-border rounded-xl border border-border">
          {accounts.map((a) => (
            <li key={a.id} className="flex items-center gap-3 px-3.5 py-3">
              <Avatar name={a.displayName} address={a.email} size={34} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-[13px] font-medium">{a.displayName ?? a.email}</p>
                <p className="truncate text-[12px] text-muted">{a.email} · {a.authKind === "oauth2" ? "OAuth2" : tr("settings:password")}</p>
                <p className={cn("text-[11.5px]", a.syncStatus === "error" ? "text-danger" : "text-faint")}>
                  {a.syncStatus === "error" ? describeStored(a.syncError) ?? tr("settings:syncError") : a.syncStatus === "syncing" ? tr("sidebar:syncing") : tr("sidebar:synced", { when: relativeTime(a.lastSyncAt) })}
                </p>
                {a.pendingCopies > 0 && <p className="text-[11.5px] text-warn" title={describeStored(a.pendingCopyError) ?? undefined}>{tr("settings:pendingCopies", { count: a.pendingCopies })}</p>}
              </div>
              {a.authKind === "oauth2" && a.syncStatus === "error" && (
                <button className={btnGhost} disabled={backend.demo} onClick={() => void backend.reconnectOAuthAccount(a.id).then(() => toast({ message: tr("settings:reconnected") })).catch((e) => toast({ message: String(e) }))}>{tr("settings:reconnect")}</button>
              )}
              <IconButton label={tr("settings:sync")} onClick={() => void backend.syncNow(a.id).then(() => toast({ message: tr("settings:syncStarted") }))}><RefreshCw size={15} /></IconButton>
              <IconButton label={tr("settings:removeAccount")} disabled={backend.demo || removing !== null} onClick={() => void remove(a.id, a.email)}><Trash2 size={15} /></IconButton>
            </li>
          ))}
          {accounts.length === 0 && <li className="px-3.5 py-6 text-center text-[13px] text-muted">{tr("settings:noAccounts")}</li>}
        </ul>
      </Section>

      {accounts.length > 0 && (
        <Section title={tr("settings:signatures")} hint={tr("settings:signaturesHint")}>
          <div className="space-y-3">
            {accounts.map((a) => <AccountSignatures key={a.id} account={a} />)}
          </div>
        </Section>
      )}

      {accounts.length > 0 && (
        <Section title={tr("settings:copySent.title")} hint={tr("settings:copySent.hint")}>
          <div className="space-y-3">
            {accounts.map((a) => <CopySentField key={a.id} account={a} />)}
          </div>
        </Section>
      )}

      {accounts.length > 0 && (
        <Section title={tr("settings:sendAddresses")} hint={tr("settings:sendAddressesHint")}>
          <div className="space-y-4">
            {accounts.map((a) => <IdentitiesField key={a.id} account={a} />)}
          </div>
        </Section>
      )}

      {!backend.demo && (
        <Section title={tr("settings:addAccount")} hint={tr("settings:addAccountHint")}>
          <AddAccountForm />
        </Section>
      )}
    </>
  );
}

// ───────────── Agent IA (MCP) ─────────────

const TOOLS = ["list_accounts", "list_folders", "search_emails", "get_thread", "summarize_thread"];

function AgentPanel() {
  const backend = useMail((s) => s.backend);
  const [info, setInfo] = useState<McpInfo | null>(null);
  useEffect(() => { backend.mcpInfo().then(setInfo, () => undefined); }, [backend]);
  const bin = info?.binaryPath ?? tr("settings:agent.pathPlaceholder");
  const json = (extra = "") => JSON.stringify({ mcpServers: { envelope: { command: bin, ...(extra ? { args: [extra] } : {}) } } }, null, 2);

  return (
    <>
      <p className="mb-5 rounded-lg bg-warn/10 px-3 py-2 text-[12.5px]"><Trans i18nKey="settings:agent.lockWarning" components={{ code: <code /> }} /></p>
      <div className="mb-7 flex items-center gap-4 rounded-2xl bg-gradient-to-br from-accent/12 to-[#8b5cf6]/10 p-5">
        <span className="inline-flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-bg text-accent shadow-card"><Bot size={24} strokeWidth={1.6} /></span>
        <div className="min-w-0 flex-1">
          <p className="text-[15px] font-semibold tracking-tight">{tr("settings:agent.headline")}</p>
          <p className="mt-0.5 text-[12.5px] text-muted">{tr("settings:agent.subline")}</p>
        </div>
        <span className={cn("shrink-0 rounded-full px-2.5 py-1 text-[11.5px] font-semibold", info?.binaryPath ? "bg-success/12 text-success" : "bg-warn/15 text-warn")}>
          {info?.binaryPath ? tr("settings:agent.ready") : tr("settings:agent.missing")}
        </span>
      </div>

      <Section title={tr("settings:agent.connect")} hint={info?.binaryPath ? undefined : tr("settings:agent.buildFirst")}>
        <div className="space-y-3">
          <Code title="Claude Code" text={`claude mcp add envelope -- ${bin}`} />
          <Code title="Claude Desktop / Cursor (mcpServers)" text={json()} />
          {info && <p className="selectable text-[12px] text-muted"><Trans i18nKey="settings:agent.dbPath" values={{ path: info.dbPath }} components={{ mono: <span className="font-mono" /> }} /></p>}
        </div>
      </Section>

      <Section title={tr("settings:agent.canDo")}>
        <ul className="divide-y divide-border rounded-xl border border-border">
          {TOOLS.map((name) => (
            <li key={name} className="flex items-center gap-3 px-3.5 py-2.5">
              <code className="rounded-md bg-hover px-1.5 py-0.5 font-mono text-[12px]">{name}</code>
              <span className="flex-1 text-[12.5px] text-muted">{tr(`settings:agent.tools.${name}`)}</span>
              <span className="rounded-full bg-success/12 px-2 py-0.5 text-[11px] font-semibold text-success">{tr("settings:agent.readOnlyBadge")}</span>
            </li>
          ))}
        </ul>
      </Section>

      <Section title={tr("settings:agent.guaranteesTitle")}>
        <ul className="space-y-2 text-[13px]">
          {["readOnly", "untrusted", "attachments", "local"].map((k) => (
            <li key={k} className="flex gap-2.5"><ShieldCheck size={16} className="mt-px shrink-0 text-success" />{tr(`settings:agent.guarantees.${k}`)}</li>
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
    <Section title={tr("settings:font.title")} hint={tr("settings:font.hint")}>
      <div className="flex max-w-[420px] flex-col gap-2.5">
        <select
          aria-label={tr("settings:font.label")}
          value={selected}
          onChange={(e) => {
            const v = e.target.value;
            if (v === "custom") setFont((customName(typed) ? `custom:${customName(typed)}` : "system") as FontPref);
            else setFont(v as FontPref);
            if (v === "custom" && !customName(typed)) setTyped("");
          }}
          className={cn(inputCls, "px-2")}
        >
          <option value="system">{tr("settings:font.system")}</option>
          {FREE_FONTS.map((f) => <option key={f.id} value={f.id}>{f.label}</option>)}
          <option value="custom">{tr("settings:font.installed")}</option>
        </select>
        {(selected === "custom" || isCustom) && (
          <div className="flex gap-2">
            <input
              aria-label={tr("settings:font.installedName")}
              className={cn(inputCls, "min-w-0 flex-1")}
              autoComplete="off"
              spellCheck={false}
              placeholder={tr("settings:font.installedPlaceholder")}
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && valid) setFont(`custom:${customName(typed)!}`); }}
            />
            <button className={btnPrimary} disabled={!valid} onClick={() => setFont(`custom:${customName(typed)!}`)}>{tr("editor:apply")}</button>
          </div>
        )}
        {note && <p className="text-[12px] text-muted">{note}</p>}
        <p className="rounded-lg bg-bg-subtle px-3 py-2.5 text-[13px] leading-relaxed">
          {tr("settings:font.sample")}
        </p>
      </div>
    </Section>
  );
}

/** Brouillons enregistrés sur le serveur : retrouvés sur un autre appareil ou dans un autre client. */
function DraftsSection() {
  const [on, setOn] = useState(serverDraftsEnabled);
  return (
    <Section title={tr("settings:drafts.title")} hint={tr("settings:drafts.hint")}>
      <label className="flex max-w-[420px] items-center gap-3 text-[13px]">
        <span className="flex-1">{tr("settings:drafts.save")}</span>
        <Switch checked={on} onChange={(v) => { setOn(v); setServerDrafts(v); }} label={tr("settings:drafts.save")} />
      </label>
    </Section>
  );
}

const systemLockLabel = () => tr(isMac ? "settings:lock.systemMac" : isWindows ? "settings:lock.systemWindows" : "settings:lock.systemOther");
const AUTO_LOCK = [0, 60, 300, 900, 1800, 3600];

/** Verrouillage de l'application : désactivé, par le système (Touch ID, Windows Hello) ou par un mot de passe. */
function LockSection() {
  const { backend, lock, setLockStatus, toast } = useMail(useShallow((s) => ({ backend: s.backend, lock: s.lock, setLockStatus: s.setLockStatus, toast: s.toast })));
  const [method, setMethod] = useState<LockMethod>(lock?.method ?? "off");
  const [timeout, setTimeoutSecs] = useState(lock?.timeoutSecs ?? 0);
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [current, setCurrent] = useState("");
  const [busy, setBusy] = useState(false);
  if (!lock) return null;
  const active = lock.method;
  const changed = method !== active || timeout !== lock.timeoutSecs || (method === "password" && password !== "");
  const passwordProblem = method === "password" && (password !== "" || active !== "password")
    ? password.length < 8 ? tr("settings:lock.min8") : password !== confirmation ? tr("settings:lock.differ") : null
    : null;

  const save = async () => {
    setBusy(true);
    try {
      const next = await backend.setLock(method, timeout, method === "password" && password ? password : undefined, active === "password" ? current : undefined);
      setLockStatus(next);
      setPassword(""); setConfirmation(""); setCurrent("");
      toast({ message: method === "off" ? tr("settings:lock.disabled") : tr("settings:lock.saved") });
    } catch (e) {
      toast({ message: tr("settings:lock.saveFailed", { error: errText(e) }), tone: "error", duration: 6000 });
    } finally {
      setBusy(false);
    }
  };

  const options: [LockMethod, string][] = [["off", tr("settings:lock.off")], ...(lock.systemAvailable ? [["system", systemLockLabel()] as [LockMethod, string]] : []), ["password", tr("settings:lock.password")]];
  return (
    <Section title={tr("settings:lock.title")} hint={tr("settings:lock.hint")}>
      <fieldset className="flex max-w-[460px] flex-col gap-2">
        <legend className="sr-only">{tr("settings:lock.method")}</legend>
        {options.map(([id, label]) => (
          <label key={id} className="flex cursor-default items-center gap-2.5 text-[13px]">
            <input type="radio" name="lock-method" checked={method === id} onChange={() => setMethod(id)} className="accent-[var(--accent)]" />
            {label}
          </label>
        ))}
        {!lock.systemAvailable && <p className="text-[12px] text-muted">{tr("settings:lock.noNative")}</p>}
      </fieldset>
      {method !== "off" && (
        <div className="mt-3 flex max-w-[460px] flex-col gap-2.5">
          <label className="flex items-center gap-2.5 text-[13px]">
            <span className="w-44 shrink-0 text-muted">{tr("settings:lock.after")}</span>
            <select aria-label={tr("settings:lock.idle")} value={timeout} onChange={(e) => setTimeoutSecs(Number(e.target.value))} className={cn(inputCls, "px-2")}>
              {AUTO_LOCK.map((secs) => <option key={secs} value={secs}>{tr(`settings:lock.after_${secs}`)}</option>)}
            </select>
          </label>
          {method === "password" && (
            <>
              <input type="password" aria-label={tr("settings:lock.newPassword")} autoComplete="new-password" placeholder={active === "password" ? tr("settings:lock.newPasswordKeep") : tr("settings:lock.passwordMin")} value={password} onChange={(e) => setPassword(e.target.value)} className={inputCls} />
              {(password !== "" || active !== "password") && <input type="password" aria-label={tr("settings:lock.confirm")} autoComplete="new-password" placeholder={tr("settings:lock.confirm")} value={confirmation} onChange={(e) => setConfirmation(e.target.value)} className={inputCls} />}
              {passwordProblem && password !== "" && <p className="text-[12px] text-danger">{passwordProblem}</p>}
            </>
          )}
        </div>
      )}
      {active === "password" && changed && (
        <input type="password" aria-label={tr("settings:lock.current")} autoComplete="current-password" placeholder={tr("settings:lock.currentConfirm")} value={current} onChange={(e) => setCurrent(e.target.value)} className={cn(inputCls, "mt-3 max-w-[460px]")} />
      )}
      {active === "system" && changed && <p className="mt-3 text-[12px] text-muted">{tr("settings:lock.systemWillAsk")}</p>}
      <button type="button" disabled={!changed || busy || passwordProblem !== null || (active === "password" && !current)} onClick={() => void save()} className={cn(btnPrimary, "mt-4")}>{tr("settings:save")}</button>
    </Section>
  );
}

function NotificationsSection() {
  const [on, setOn] = useState(notificationsEnabled);
  const toast = useMail((s) => s.toast);
  const test = async () => {
    try {
      await sendNative({ title: "Envelope", body: tr("settings:notifications.works") });
    } catch (e) {
      toast({ message: tr("settings:notifications.failed", { error: String(e) }), tone: "error", duration: 6000 });
    }
  };
  return (
    <Section title={tr("settings:notifications.title")} hint={tr("settings:notifications.hint")}>
      <label className="flex max-w-[420px] items-center gap-3 text-[13px]">
        <span className="flex-1">{tr("settings:notifications.notify")}</span>
        <Switch checked={on} onChange={(v) => { setOn(v); setNotificationsEnabled(v); }} label={tr("settings:notifications.notify")} />
      </label>
      <button className="press mt-3 h-8 rounded-lg border border-border bg-bg px-3 text-[12.5px] font-medium hover:bg-hover" onClick={() => void test()}>{tr("settings:notifications.test")}</button>
    </Section>
  );
}

/** Langue de l'interface : le système par défaut, ou une langue livrée. */
function LanguageSection() {
  const { t } = useTranslation("language");
  const [value, setValue] = useState<LangPref>(() => parseLangPref(pref.get("lang", "system")));
  return (
    <Section title={t("title")} hint={t("hint")}>
      <Segmented<LangPref> value={value} onChange={(v) => { setValue(v); void setLanguage(v); }} options={[{ value: "system", label: t("system"), icon: Monitor }, ...LANGUAGES.map((l) => ({ value: l.code as LangPref, label: l.label }))]} />
    </Section>
  );
}

function AppearancePanel() {
  const s = useMail();
  return (
    <>
      <LanguageSection />
      <Section title={tr("settings:appearance.theme")}>
        <Segmented<ThemePref> value={s.theme} onChange={s.setTheme} options={[{ value: "system", label: tr("language:system"), icon: Monitor }, { value: "light", label: tr("settings:appearance.light"), icon: Sun }, { value: "dark", label: tr("settings:appearance.dark"), icon: Moon }]} />
      </Section>
      <Section title={tr("settings:appearance.colors")} hint={tr("settings:appearance.colorsHint")}>
        <Segmented<ColorsPref> value={s.colors} onChange={s.setColors} options={[{ value: "system", label: tr("language:system"), icon: Monitor }, { value: "envelope", label: "Envelope", icon: Palette }]} />
      </Section>
      <Section title={tr("settings:appearance.density")}>
        <Segmented value={s.density} onChange={s.setDensity} options={[{ value: "comfortable", label: tr("settings:appearance.comfortable") }, { value: "compact", label: tr("settings:appearance.compact") }]} />
      </Section>
      <FontSection />
      <DraftsSection />
      <NotificationsSection />
      <Section title={tr("settings:appearance.previews")}>
        <label className="flex max-w-[420px] items-center gap-3 text-[13px]">
          <span className="flex-1">{tr("settings:appearance.previewsLabel")}</span>
          <Switch checked={s.previews} onChange={s.setPreviews} label={tr("settings:appearance.previewsSwitch")} />
        </label>
      </Section>
    </>
  );
}

function AboutPanel() {
  const [version, setVersion] = useState<string | null>(null);
  const [auto, setAuto] = useState(autoUpdateEnabled);
  const { available, status, checkedAt, error } = useUpdate();
  useEffect(() => {
    if (isTauri()) void import("@tauri-apps/api/app").then(({ getVersion }) => getVersion()).then(setVersion);
  }, []);
  return (
    <>
      <Section title={tr("settings:about.version")}>
        <p className="text-[13px]">{version ? `Envelope ${version}` : tr("settings:about.browserPreview")}</p>
      </Section>
      <Section title={tr("settings:about.updates")} hint={tr("settings:about.updatesHint")}>
        <label className="flex max-w-[420px] items-center gap-3 text-[13px]">
          <span className="flex-1">{tr("settings:about.autoCheck")}</span>
          <Switch checked={auto} onChange={(v) => { setAuto(v); setAutoUpdateEnabled(v); }} label={tr("settings:about.autoCheck")} />
        </label>
        <div className="mt-3 flex flex-wrap items-center gap-3">
          <button className={cn(btnGhost, "h-8 gap-1.5 px-3 text-[12.5px]")} disabled={!isTauri() || status !== "idle"} onClick={() => void checkForUpdate()}>
            {status === "checking" ? <Spinner size={12} /> : <RefreshCw size={13} />} {tr("settings:about.checkNow")}
          </button>
          {available ? (
            <button className={cn(btnPrimary, "h-8 text-[12.5px]")} disabled={status !== "idle"} onClick={() => void installUpdate()}>
              {status === "installing" ? <Spinner size={12} /> : <Download size={13} />} {tr("settings:about.install", { version: available })}
            </button>
          ) : (
            checkedAt && !error && <span className="text-[12.5px] text-muted">{tr("settings:about.upToDate", { when: relativeTime(checkedAt) })}</span>
          )}
        </div>
        {error && <p className="mt-2 text-[12.5px] text-danger">{error}</p>}
      </Section>
    </>
  );
}

// ───────────── Fenêtre ─────────────

export function Settings() {
  const { settingsTab, openSettings } = useMail(useShallow((s) => ({ settingsTab: s.settingsTab, openSettings: s.openSettings })));
  return (
    <DialogFrame open={settingsTab !== null} onOpenChange={(o) => !o && openSettings(null)} onOpenAutoFocus={(e) => e.preventDefault()} className="flex h-[min(640px,88vh)] w-[min(900px,94vw)] overflow-hidden">
          <nav className="w-[196px] shrink-0 border-r border-border bg-sidebar p-3" aria-label={settingsLabel()}>
            <Dialog.Title className="px-2.5 pb-3 pt-1 text-[15px] font-semibold tracking-tight">{settingsLabel()}</Dialog.Title>
            {TABS.map((t) => (
              <button key={t.id} onClick={() => openSettings(t.id)} aria-current={settingsTab === t.id} className={cn("press flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-[13px] font-medium", settingsTab === t.id ? "bg-active text-fg" : "text-muted hover:bg-hover hover:text-fg")}>
                <t.icon size={15} /> {t.label}
              </button>
            ))}
          </nav>
          <div className="relative min-w-0 flex-1 overflow-y-auto p-7">
            <Dialog.Close asChild><IconButton label={tr("common:close")} className="absolute right-3 top-3"><X size={16} /></IconButton></Dialog.Close>
            <h2 className="mb-5 text-[18px] font-semibold tracking-tight">{TABS.find((t) => t.id === settingsTab)?.label}</h2>
            {settingsTab === "accounts" && <AccountsPanel />}
            {settingsTab === "rules" && <RulesPanel />}
            {settingsTab === "security" && <LockSection />}
            {settingsTab === "agent" && <AgentPanel />}
            {settingsTab === "appearance" && <AppearancePanel />}
            {settingsTab === "shortcuts" && <ShortcutList />}
            {settingsTab === "about" && <AboutPanel />}
          </div>
    </DialogFrame>
  );
}
