import { Pencil, Play, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Rule, RuleMove } from "@/lib/api";
import { useMail } from "@/lib/store";
import { cn, errText } from "@/lib/utils";
import { btnGhost, btnPrimary, IconButton, inputCls } from "./ui/primitives";

const MOVES: { value: RuleMove | ""; label: string }[] = [
  { value: "", label: "Ne pas déplacer" },
  { value: "archive", label: "Archiver" },
  { value: "trash", label: "Mettre à la corbeille" },
  { value: "junk", label: "Marquer comme spam" },
];
const EMPTY: Rule = { id: 0, name: "", enabled: true, accountId: null, query: "", markRead: false, star: false, moveTo: null };

/** Résumé des actions d'une règle, pour la liste. */
const actionsLabel = (r: Rule) => [r.markRead && "marquer lu", r.star && "suivre", r.moveTo && MOVES.find((m) => m.value === r.moveTo)?.label.toLowerCase()].filter(Boolean).join(", ");

/** Règles locales : condition en syntaxe de recherche, actions sur les nouveaux messages de la boîte de réception. */
export function RulesPanel() {
  const { backend, accounts, toast, reload } = useMail(useShallow((s) => ({ backend: s.backend, accounts: s.accounts, toast: s.toast, reload: s.reload })));
  const [rules, setRules] = useState<Rule[]>([]);
  const [editing, setEditing] = useState<Rule | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const refresh = () => backend.listRules().then(setRules, (e) => toast({ message: `Règles illisibles — ${errText(e)}`, tone: "error" }));
  useEffect(() => { void refresh(); }, [backend]); // eslint-disable-line react-hooks/exhaustive-deps

  const save = async (r: Rule) => {
    try { await backend.saveRule(r); setEditing(null); await refresh(); } catch (e) { toast({ message: `Règle non enregistrée — ${errText(e)}`, tone: "error" }); }
  };
  const remove = (r: Rule) => backend.deleteRule(r.id).then(refresh, (e) => toast({ message: `Suppression impossible — ${errText(e)}`, tone: "error" }));
  const runNow = async () => {
    setConfirming(false);
    setBusy(true);
    try {
      const n = await backend.runRulesNow();
      toast({ message: n ? `${n} conversation${n > 1 ? "s" : ""} traitée${n > 1 ? "s" : ""}` : "Aucune conversation ne correspond" });
      if (n) void reload({ silent: true });
    } catch (e) { toast({ message: `Application impossible — ${errText(e)}`, tone: "error" }); } finally { setBusy(false); }
  };

  return (
    <>
      <p className="mb-4 text-[12.5px] text-muted">Les règles s'appliquent aux nouveaux messages non lus de la boîte de réception, quand Envelope est ouvert ; elles agissent sur la conversation entière. La condition utilise la syntaxe de la recherche (<code>de:alice</code>, <code>objet:facture</code>, <code>pj</code>, un mot…). Elles agissent aussi sur votre boîte côté serveur (archivage, corbeille, spam).</p>
      <ul className="divide-y divide-border rounded-xl border border-border">
        {rules.map((r) => (
          <li key={r.id} className="flex items-center gap-3 px-3.5 py-2.5">
            <input type="checkbox" aria-label={`Activer « ${r.name} »`} checked={r.enabled} onChange={(e) => void save({ ...r, enabled: e.target.checked })} />
            <div className={cn("min-w-0 flex-1", !r.enabled && "opacity-55")}>
              <div className="truncate text-[13px] font-medium">{r.name}</div>
              <div className="truncate text-[12px] text-muted"><span className="font-mono">{r.query}</span> → {actionsLabel(r)}{r.accountId ? ` · ${accounts.find((a) => a.id === r.accountId)?.email ?? "compte supprimé"}` : ""}</div>
            </div>
            <IconButton label="Modifier" onClick={() => setEditing(r)}><Pencil size={15} /></IconButton>
            <IconButton label="Supprimer" onClick={() => void remove(r)}><Trash2 size={15} className="text-danger" /></IconButton>
          </li>
        ))}
        {rules.length === 0 && <li className="px-3.5 py-6 text-center text-[13px] text-muted">Aucune règle pour l'instant.</li>}
      </ul>
      <div className="mt-3 flex gap-2">
        <button className={`${btnPrimary} gap-2`} onClick={() => setEditing({ ...EMPTY })}><Plus size={14} /> Nouvelle règle</button>
        {confirming ? (
          <>
            <button className={`${btnPrimary} gap-2`} onClick={() => void runNow()}>Confirmer : agir sur le serveur</button>
            <button className={btnGhost} onClick={() => setConfirming(false)}>Annuler</button>
          </>
        ) : (
          <button className={`${btnGhost} gap-2`} disabled={busy || rules.length === 0} onClick={() => setConfirming(true)}><Play size={14} /> Appliquer aux messages non lus de la boîte de réception</button>
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
      <h3 className="text-[13px] font-semibold">{r.id ? "Modifier la règle" : "Nouvelle règle"}</h3>
      <label className="block text-[12.5px]">Nom<input className={`${inputCls} mt-1`} value={r.name} maxLength={100} onChange={(e) => set({ name: e.target.value })} autoFocus /></label>
      <label className="block text-[12.5px]">Condition (syntaxe de recherche)<input className={`${inputCls} mt-1 font-mono`} value={r.query} maxLength={500} placeholder="de:newsletter objet:promo" onChange={(e) => set({ query: e.target.value })} /></label>
      <label className="block text-[12.5px]">Compte
        <select className={`${inputCls} mt-1`} value={r.accountId ?? ""} onChange={(e) => set({ accountId: e.target.value || null })}>
          <option value="">Tous les comptes</option>
          {accounts.map((a) => <option key={a.id} value={a.id}>{a.email}</option>)}
        </select>
      </label>
      <fieldset className="space-y-1.5 text-[13px]">
        <legend className="mb-1 text-[12.5px]">Actions</legend>
        <label className="flex items-center gap-2"><input type="checkbox" checked={r.markRead} onChange={(e) => set({ markRead: e.target.checked })} /> Marquer comme lu</label>
        <label className="flex items-center gap-2"><input type="checkbox" checked={r.star} onChange={(e) => set({ star: e.target.checked })} /> Suivre (étoile)</label>
        <select aria-label="Déplacer" className={inputCls} value={r.moveTo ?? ""} onChange={(e) => set({ moveTo: (e.target.value || null) as RuleMove | null })}>
          {MOVES.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
        </select>
      </fieldset>
      <div className="flex justify-end gap-2">
        <button type="button" className={btnGhost} onClick={onCancel}>Annuler</button>
        <button type="submit" className={btnPrimary} disabled={!valid}>Enregistrer</button>
      </div>
    </form>
  );
}
