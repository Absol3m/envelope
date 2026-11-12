import { Pencil, Play, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Rule, RuleMove } from "@/lib/api";
import { useMail } from "@/lib/store";
import { cn, errText } from "@/lib/utils";
import { btnGhost, btnPrimary, IconButton, inputCls } from "./ui/primitives";
import { t as tr } from "@/lib/i18n";
import { Trans } from "react-i18next";

const moves = (): { value: RuleMove | ""; label: string }[] => [
  { value: "", label: tr("rules:moves.none") },
  { value: "archive", label: tr("rules:moves.archive") },
  { value: "trash", label: tr("rules:moves.trash") },
  { value: "junk", label: tr("rules:moves.junk") },
];
const EMPTY: Rule = { id: 0, name: "", enabled: true, accountId: null, query: "", markRead: false, star: false, moveTo: null };

/** Résumé des actions d'une règle, pour la liste. */
const actionsLabel = (r: Rule) => [r.markRead && tr("rules:summary.markRead"), r.star && tr("rules:summary.star"), r.moveTo && moves().find((m) => m.value === r.moveTo)?.label.toLowerCase()].filter(Boolean).join(", ");

/** Règles locales : condition en syntaxe de recherche, actions sur les nouveaux messages de la boîte de réception. */
export function RulesPanel() {
  const { backend, accounts, toast, reload } = useMail(useShallow((s) => ({ backend: s.backend, accounts: s.accounts, toast: s.toast, reload: s.reload })));
  const [rules, setRules] = useState<Rule[]>([]);
  const [editing, setEditing] = useState<Rule | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const refresh = () => backend.listRules().then(setRules, (e) => toast({ message: tr("rules:loadFailed", { error: errText(e) }), tone: "error" }));
  useEffect(() => { void refresh(); }, [backend]); // eslint-disable-line react-hooks/exhaustive-deps

  const save = async (r: Rule) => {
    try { await backend.saveRule(r); setEditing(null); await refresh(); } catch (e) { toast({ message: tr("rules:saveFailed", { error: errText(e) }), tone: "error" }); }
  };
  const remove = (r: Rule) => backend.deleteRule(r.id).then(refresh, (e) => toast({ message: tr("rules:deleteFailed", { error: errText(e) }), tone: "error" }));
  const runNow = async () => {
    setConfirming(false);
    setBusy(true);
    try {
      const n = await backend.runRulesNow();
      toast({ message: n ? tr("rules:processed", { count: n }) : tr("rules:noMatch") });
      if (n) void reload({ silent: true });
    } catch (e) { toast({ message: tr("rules:applyFailed", { error: errText(e) }), tone: "error" }); } finally { setBusy(false); }
  };

  return (
    <>
      <p className="mb-4 text-[12.5px] text-muted"><Trans i18nKey="rules:intro" components={{ code: <code /> }} /></p>
      <ul className="divide-y divide-border rounded-xl border border-border">
        {rules.map((r) => (
          <li key={r.id} className="flex items-center gap-3 px-3.5 py-2.5">
            <input type="checkbox" aria-label={tr("rules:enable", { name: r.name })} checked={r.enabled} onChange={(e) => void save({ ...r, enabled: e.target.checked })} />
            <div className={cn("min-w-0 flex-1", !r.enabled && "opacity-55")}>
              <div className="truncate text-[13px] font-medium">{r.name}</div>
              <div className="truncate text-[12px] text-muted"><span className="font-mono">{r.query}</span> → {actionsLabel(r)}{r.accountId ? ` · ${accounts.find((a) => a.id === r.accountId)?.email ?? tr("rules:accountGone")}` : ""}</div>
            </div>
            <IconButton label={tr("scheduled:edit")} onClick={() => setEditing(r)}><Pencil size={15} /></IconButton>
            <IconButton label={tr("rules:delete")} onClick={() => void remove(r)}><Trash2 size={15} className="text-danger" /></IconButton>
          </li>
        ))}
        {rules.length === 0 && <li className="px-3.5 py-6 text-center text-[13px] text-muted">{tr("rules:none")}</li>}
      </ul>
      <div className="mt-3 flex gap-2">
        <button className={`${btnPrimary} gap-2`} onClick={() => setEditing({ ...EMPTY })}><Plus size={14} /> {tr("rules:new")}</button>
        {confirming ? (
          <>
            <button className={`${btnPrimary} gap-2`} onClick={() => void runNow()}>{tr("rules:confirmRun")}</button>
            <button className={btnGhost} onClick={() => setConfirming(false)}>{tr("composer:cancel")}</button>
          </>
        ) : (
          <button className={`${btnGhost} gap-2`} disabled={busy || rules.length === 0} onClick={() => setConfirming(true)}><Play size={14} /> {tr("rules:runNow")}</button>
        )}
      </div>
      {editing && <RuleForm key={editing.id} rule={editing} accounts={accounts.filter((a) => a.id !== "local")} onCancel={() => setEditing(null)} onSave={save} />}
    </>
  );
}

function RuleForm({ rule, accounts, onSave, onCancel }: { rule: Rule; accounts: { id: string; email: string }[]; onSave(r: Rule): void; onCancel(): void }) {
  const [r, setR] = useState(rule);
  const set = (patch: Partial<Rule>) => setR((cur) => ({ ...cur, ...patch }));
  const valid = r.name.trim() !== "" && r.query.trim() !== "" && (r.markRead || r.star || r.moveTo !== null);
  return (
    <form className="mt-4 space-y-3 rounded-xl border border-border p-4" onSubmit={(e) => { e.preventDefault(); if (valid) onSave(r); }}>
      <h3 className="text-[13px] font-semibold">{r.id ? tr("rules:editRule") : tr("rules:new")}</h3>
      <label className="block text-[12.5px]">{tr("rules:name")}<input className={`${inputCls} mt-1`} value={r.name} maxLength={100} onChange={(e) => set({ name: e.target.value })} autoFocus /></label>
      <label className="block text-[12.5px]">{tr("rules:condition")}<input className={`${inputCls} mt-1 font-mono`} value={r.query} maxLength={500} placeholder={tr("rules:conditionPlaceholder")} onChange={(e) => set({ query: e.target.value })} /></label>
      <label className="block text-[12.5px]">{tr("rules:account")}
        <select className={`${inputCls} mt-1`} value={r.accountId ?? ""} onChange={(e) => set({ accountId: e.target.value || null })}>
          <option value="">{tr("rules:allAccounts")}</option>
          {accounts.map((a) => <option key={a.id} value={a.id}>{a.email}</option>)}
        </select>
      </label>
      <fieldset className="space-y-1.5 text-[13px]">
        <legend className="mb-1 text-[12.5px]">{tr("rules:actions")}</legend>
        <label className="flex items-center gap-2"><input type="checkbox" checked={r.markRead} onChange={(e) => set({ markRead: e.target.checked })} /> {tr("topbar:markRead")}</label>
        <label className="flex items-center gap-2"><input type="checkbox" checked={r.star} onChange={(e) => set({ star: e.target.checked })} /> {tr("rules:starAction")}</label>
        <select aria-label={tr("rules:move")} className={inputCls} value={r.moveTo ?? ""} onChange={(e) => set({ moveTo: (e.target.value || null) as RuleMove | null })}>
          {moves().map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
        </select>
      </fieldset>
      <div className="flex justify-end gap-2">
        <button type="button" className={btnGhost} onClick={onCancel}>{tr("composer:cancel")}</button>
        <button type="submit" className={btnPrimary} disabled={!valid}>{tr("settings:save")}</button>
      </div>
    </form>
  );
}
