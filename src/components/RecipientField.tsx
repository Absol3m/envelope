import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Addr, Backend } from "@/lib/api";
import { parseRecipients, type Recipient } from "@/lib/compose";
import { cn } from "@/lib/utils";

interface Props {
  label: string;
  value: Recipient[];
  onChange: (next: Recipient[]) => void;
  backend: Backend;
  accountId: string;
  autoFocus?: boolean;
}

/** Champ de destinataires : pastilles, saisie libre (virgule, point-virgule, collage) et suggestions locales. */
export function RecipientField({ label, value, onChange, backend, accountId, autoFocus }: Props) {
  const [text, setText] = useState("");
  const [invalid, setInvalid] = useState(false);
  const [options, setOptions] = useState<Addr[]>([]);
  const [active, setActive] = useState(0);
  const ref = useRef<HTMLInputElement>(null);

  const add = (list: Recipient[]) => {
    const known = new Set(value.map((r) => r.address.toLowerCase()));
    const fresh = list.filter((r) => !known.has(r.address.toLowerCase()) && known.add(r.address.toLowerCase()));
    if (fresh.length) onChange([...value, ...fresh]);
  };
  /** Saisie en cours → pastilles ; ce qui n'est pas une adresse reste dans le champ, en rouge. */
  const commit = (raw = text) => {
    if (!raw.trim()) return;
    const { ok, rejected } = parseRecipients(raw);
    add(ok);
    setText(rejected.join(", "));
    setInvalid(rejected.length > 0);
    setOptions([]);
  };

  useEffect(() => {
    const q = text.trim();
    if (q.length < 2 || q.includes("<")) return setOptions([]);
    let stale = false;
    const t = setTimeout(() => {
      backend.suggestContacts(accountId, q).then((r) => {
        if (stale) return;
        const known = new Set(value.map((v) => v.address.toLowerCase()));
        setOptions(r.filter((a) => !known.has(a.address.toLowerCase())));
        setActive(0);
      }).catch(() => {});
    }, 120);
    return () => { stale = true; clearTimeout(t); };
  }, [text, accountId, backend]); // eslint-disable-line react-hooks/exhaustive-deps

  const pick = (a: Addr) => {
    add([{ address: a.address, name: a.name }]);
    setText("");
    setOptions([]);
    setInvalid(false);
  };

  return (
    <div className="relative flex min-h-[38px] items-start gap-2 border-b border-border px-4 py-1.5" onClick={() => ref.current?.focus()}>
      <span className="w-6 shrink-0 pt-[5px] text-[12px] text-muted">{label}</span>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
        {value.map((r) => (
          <span key={r.address} className="inline-flex max-w-full items-center gap-1 rounded-full bg-accent/10 py-0.5 pl-2.5 pr-1 text-[12.5px] text-fg" title={r.address}>
            <span className="truncate">{r.name || r.address}</span>
            <button type="button" aria-label={`Retirer ${r.address}`} className="press rounded-full p-0.5 text-muted hover:bg-hover hover:text-fg" onClick={(e) => { e.stopPropagation(); onChange(value.filter((v) => v.address !== r.address)); }}>
              <X size={11} />
            </button>
          </span>
        ))}
        <input
          ref={ref}
          autoFocus={autoFocus}
          aria-label={label}
          autoComplete="off"
          spellCheck={false}
          value={text}
          className={cn("min-w-[120px] flex-1 bg-transparent py-1 text-[13.5px] outline-none", invalid && "text-danger")}
          onChange={(e) => { setText(e.target.value); setInvalid(false); }}
          onBlur={() => commit()}
          onPaste={(e) => {
            const pasted = e.clipboardData.getData("text");
            if (/[,;\n<]/.test(pasted)) { e.preventDefault(); commit(text + pasted); }
          }}
          onKeyDown={(e) => {
            if (e.metaKey || e.ctrlKey) {
              if (e.key === "Enter") commit(); // ⌘↵ : la saisie en cours compte, puis l'envoi se poursuit
              return;
            }
            if (options.length && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
              e.preventDefault();
              setActive((i) => (i + (e.key === "ArrowDown" ? 1 : options.length - 1)) % options.length);
            } else if ((e.key === "Enter" || e.key === "Tab") && options.length && text.trim()) {
              e.preventDefault();
              pick(options[active]);
            } else if (e.key === "Enter" || e.key === "," || e.key === ";") {
              e.preventDefault();
              commit();
            } else if (e.key === "Escape" && options.length) {
              e.preventDefault();
              e.stopPropagation();
              setOptions([]);
            } else if (e.key === "Backspace" && !text && value.length) {
              onChange(value.slice(0, -1));
            }
          }}
        />
      </div>
      {options.length > 0 && (
        <ul role="listbox" className="absolute left-12 top-full z-10 mt-1 w-[min(380px,80%)] overflow-hidden rounded-xl bg-elevated p-1 shadow-pop">
          {options.map((a, i) => (
            <li key={a.address} role="option" aria-selected={i === active}>
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(a)}
                className={cn("flex w-full flex-col rounded-lg px-2.5 py-1.5 text-left", i === active && "bg-hover")}
              >
                <span className="truncate text-[13px] font-medium">{a.name || a.address}</span>
                {a.name && <span className="truncate text-[11.5px] text-muted">{a.address}</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
