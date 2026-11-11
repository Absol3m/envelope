import { Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Backend } from "@/lib/api";
import { applySuggestion, contextAt, operatorItems, valueItems, type Suggestion } from "@/lib/searchSuggest";
import { cn } from "@/lib/utils";
import { Kbd } from "./ui/primitives";

/** Barre de recherche avec suggestions : opérateurs, puis valeurs (contacts, dates). Tab ou Entrée (après ↑/↓) accepte. */
export function SearchBox({ query, setQuery, backend, accountId }: {
  query: string;
  setQuery(q: string): void;
  backend: Backend;
  accountId: string | null;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [caret, setCaret] = useState(0);
  const [focused, setFocused] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const [active, setActive] = useState(0);
  const [navigated, setNavigated] = useState(false);
  const [contacts, setContacts] = useState<Suggestion[]>([]);

  // « / » focalise la recherche (événement émis par le gestionnaire clavier global).
  useEffect(() => {
    const focus = () => inputRef.current?.focus();
    window.addEventListener("envelope:focus-search", focus);
    return () => window.removeEventListener("envelope:focus-search", focus);
  }, []);

  const ctx = useMemo(() => contextAt(query, caret), [query, caret]);
  const contactPrefix = ctx?.kind === "contact" ? ctx.prefix : null;

  // Contacts connus (même source que la composition), avec une courte attente pendant la frappe.
  useEffect(() => {
    if (!contactPrefix) return setContacts([]);
    let stale = false;
    const t = setTimeout(() => {
      backend.suggestContacts(accountId, contactPrefix).then((r) => {
        if (!stale) setContacts(r.map((a) => ({ label: a.name || a.address, hint: a.name ? a.address : undefined, insert: `${a.address} ` })));
      }).catch(() => {});
    }, 120);
    return () => { stale = true; clearTimeout(t); };
  }, [backend, accountId, contactPrefix]);

  const items: Suggestion[] = useMemo(() => {
    if (!ctx) return [];
    if (ctx.kind === "operator") {
      const found = operatorItems(ctx.prefix);
      // Opérateur déjà complet (« pj ») : rien à proposer.
      return found.length === 1 && found[0].insert.trim() === ctx.token.text ? [] : found;
    }
    return ctx.kind === "contact" ? contacts : valueItems(ctx.op, ctx.prefix);
  }, [ctx, contacts]);

  useEffect(() => { setActive(0); setNavigated(false); setDismissed(false); }, [query]);
  const open = focused && !dismissed && items.length > 0;

  const accept = (item: Suggestion) => {
    if (!ctx) return;
    const lead = ctx.kind === "operator" ? "" : ctx.token.text.slice(0, ctx.token.text.indexOf(":") + 1);
    const { text, caret: at } = applySuggestion(query, ctx.token, lead, item.insert);
    setQuery(text);
    setCaret(at);
    requestAnimationFrame(() => inputRef.current?.setSelectionRange(at, at));
  };

  return (
    <div className="relative">
      <label className="flex h-8 items-center gap-2 rounded-lg border border-border bg-bg px-2.5 text-muted transition-colors focus-within:border-accent/60 focus-within:ring-[3px] focus-within:ring-accent-soft">
        <Search size={14} className="shrink-0" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => { setQuery(e.target.value); setCaret(e.target.selectionStart ?? e.target.value.length); }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={(e) => {
            if (open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
              e.preventDefault();
              setNavigated(true);
              setActive((i) => (i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length);
            } else if (open && ((e.key === "Tab" && !e.shiftKey) || (e.key === "Enter" && navigated))) {
              e.preventDefault();
              accept(items[Math.min(active, items.length - 1)]);
            } else if (e.key === "Escape") {
              if (open) { e.preventDefault(); e.stopPropagation(); setDismissed(true); return; }
              setQuery("");
              e.currentTarget.blur();
            }
          }}
          placeholder="Rechercher (de: à: objet: avant: pj…)"
          aria-label="Rechercher"
          role="combobox"
          aria-expanded={open}
          aria-controls="search-suggestions"
          aria-autocomplete="list"
          className="selectable min-w-0 flex-1 bg-transparent text-[13px] text-fg outline-none placeholder:text-faint"
        />
        {query ? (
          <button aria-label="Effacer la recherche" onClick={() => setQuery("")} className="press rounded p-0.5 hover:bg-hover">
            <X size={13} />
          </button>
        ) : (
          <Kbd>/</Kbd>
        )}
      </label>
      {open && (
        <ul id="search-suggestions" role="listbox" className="anim-fade-in absolute right-0 top-full z-30 mt-1 max-h-[280px] w-[340px] max-w-[90vw] overflow-y-auto rounded-xl border border-border bg-elevated p-1 shadow-pop">
          {items.map((it, i) => (
            <li key={it.insert + it.label} role="option" aria-selected={i === active}>
              <button
                // mousedown : le champ garde le focus (sinon le menu se ferme avant le clic).
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => accept(it)}
                onMouseEnter={() => setActive(i)}
                className={cn("flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[13px]", i === active ? "bg-active" : "hover:bg-hover")}
              >
                <span className="font-medium">{it.label}</span>
                {it.hint && <span className="min-w-0 flex-1 truncate text-[12px] text-muted">{it.hint}</span>}
                {i === active && <Kbd>Tab</Kbd>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
