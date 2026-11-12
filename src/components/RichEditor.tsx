import * as Dropdown from "@radix-ui/react-dropdown-menu";
import { Color } from "@tiptap/extension-color";
import Placeholder from "@tiptap/extension-placeholder";
import TextAlign from "@tiptap/extension-text-align";
import { TextStyle } from "@tiptap/extension-text-style";
import { EditorContent, useEditor, useEditorState, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { AlignCenter, AlignLeft, AlignRight, Bold, Heading2, Italic, Link2, List, ListOrdered, Palette, Quote, Redo2, RemoveFormatting, Strikethrough, Underline, Undo2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import { isEmail, isNeutralColor } from "@/lib/compose";
import { cn } from "@/lib/utils";
import { isMac } from "@/lib/platform";
import { btnGhost, IconButton } from "./ui/primitives";

/** Couleur de texte : tout HTML qui entre dans l'éditeur (collage, citation, brouillon) perd ses
 *  couleurs neutres (le noir ou le blanc de sa source) et prend celle du thème, lisible en sombre
 *  comme en clair. Les vraies couleurs (rouge, bleu…) sont gardées. */
let probe: HTMLElement | undefined;
export const ThemedColor = Color.extend({
  addGlobalAttributes() {
    return (this.parent?.() ?? []).map((g) => {
      const color = g.attributes.color;
      const parse = color?.parseHTML;
      if (!parse) return g;
      const parseHTML = (el: HTMLElement) => {
        const v = parse(el);
        if (typeof v !== "string") return v;
        // Valeur brute du style (`#000`, `black`…) ramenée à la forme normalisée (`rgb(0, 0, 0)`).
        probe ??= document.createElement("span");
        probe.style.color = "";
        probe.style.color = v;
        return isNeutralColor(probe.style.color) ? null : v;
      };
      return { ...g, attributes: { ...g.attributes, color: { ...color, parseHTML } } };
    });
  },
});

const COLORS = ["#15151a", "#dc2626", "#ea580c", "#ca8a04", "#16a34a", "#0891b2", "#2563eb", "#7c3aed", "#6b6b78"];

/** Adresse saisie → URL autorisée (http, https, mailto) ou null. Jamais de `javascript:`. */
export function normalizeUrl(input: string): string | null {
  const v = input.trim();
  if (!v) return null;
  if (/^mailto:/i.test(v)) return v;
  if (isEmail(v)) return `mailto:${v}`;
  const withScheme = /^[a-z][a-z0-9+.-]*:/i.test(v) ? v : `https://${v}`;
  try {
    const u = new URL(withScheme);
    return u.protocol === "http:" || u.protocol === "https:" ? u.href : null;
  } catch { return null; }
}

/** Les boutons ne doivent pas voler le focus : la sélection du texte reste active dans l'éditeur. */
const keepFocus = { onMouseDown: (e: React.MouseEvent) => e.preventDefault() };

function Sep() {
  return <span aria-hidden className="mx-1 h-4 w-px shrink-0 bg-border" />;
}

function Tool({ label, kbd, active, disabled, onClick, children }: { label: string; kbd?: string; active?: boolean; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <IconButton label={label} kbd={kbd} active={active} disabled={disabled} tooltipSide="top" className={cn(active && "bg-accent-soft")} onClick={onClick} {...keepFocus}>
      {children}
    </IconButton>
  );
}

function FormatBar({ editor }: { editor: Editor }) {
  const [linkOpen, setLinkOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [linkError, setLinkError] = useState(false);
  const st = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive("bold"), italic: e.isActive("italic"), underline: e.isActive("underline"), strike: e.isActive("strike"),
      h2: e.isActive("heading", { level: 2 }), bullet: e.isActive("bulletList"), ordered: e.isActive("orderedList"), quote: e.isActive("blockquote"),
      link: e.isActive("link"), left: e.isActive({ textAlign: "left" }), center: e.isActive({ textAlign: "center" }), right: e.isActive({ textAlign: "right" }),
      canUndo: e.can().undo(), canRedo: e.can().redo(),
    }),
  });
  const chain = () => editor.chain().focus();

  const openLink = () => {
    setUrl((editor.getAttributes("link").href as string | undefined) ?? "");
    setLinkError(false);
    setLinkOpen(true);
  };
  const applyLink = () => {
    if (!url.trim()) { chain().extendMarkRange("link").unsetLink().run(); return setLinkOpen(false); }
    const href = normalizeUrl(url);
    if (!href) return setLinkError(true);
    chain().extendMarkRange("link").setLink({ href }).run();
    setLinkOpen(false);
  };

  if (linkOpen) {
    return (
      <div className="flex h-10 shrink-0 items-center gap-2 border-t border-border px-3">
        <Link2 size={15} className="shrink-0 text-muted" />
        <input
          autoFocus
          aria-label="Adresse du lien"
          placeholder="https://… ou adresse e-mail"
          value={url}
          spellCheck={false}
          onChange={(e) => { setUrl(e.target.value); setLinkError(false); }}
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); applyLink(); }
            else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); setLinkOpen(false); editor.commands.focus(); }
          }}
          className={cn("min-w-0 flex-1 bg-transparent text-[13px] outline-none", linkError && "text-danger")}
        />
        {linkError && <span className="text-[11.5px] text-danger">Adresse invalide</span>}
        <button type="button" className={btnGhost} onClick={applyLink}>{url.trim() ? "Appliquer" : "Retirer le lien"}</button>
      </div>
    );
  }

  return (
    <div className="flex h-10 shrink-0 items-center gap-0.5 overflow-x-auto border-t border-border px-2" role="toolbar" aria-label="Mise en forme">
      <Tool label="Gras" kbd="mod+b" active={st.bold} onClick={() => chain().toggleBold().run()}><Bold size={15} /></Tool>
      <Tool label="Italique" kbd="mod+i" active={st.italic} onClick={() => chain().toggleItalic().run()}><Italic size={15} /></Tool>
      <Tool label="Souligné" kbd="mod+u" active={st.underline} onClick={() => chain().toggleUnderline().run()}><Underline size={15} /></Tool>
      <Tool label="Barré" active={st.strike} onClick={() => chain().toggleStrike().run()}><Strikethrough size={15} /></Tool>
      <Sep />
      <Tool label="Titre" active={st.h2} onClick={() => chain().toggleHeading({ level: 2 }).run()}><Heading2 size={15} /></Tool>
      <Tool label="Liste à puces" active={st.bullet} onClick={() => chain().toggleBulletList().run()}><List size={15} /></Tool>
      <Tool label="Liste numérotée" active={st.ordered} onClick={() => chain().toggleOrderedList().run()}><ListOrdered size={15} /></Tool>
      <Tool label="Citation" active={st.quote} onClick={() => chain().toggleBlockquote().run()}><Quote size={15} /></Tool>
      <Sep />
      <Tool label="Aligner à gauche" active={st.left} onClick={() => chain().setTextAlign("left").run()}><AlignLeft size={15} /></Tool>
      <Tool label="Centrer" active={st.center} onClick={() => chain().setTextAlign("center").run()}><AlignCenter size={15} /></Tool>
      <Tool label="Aligner à droite" active={st.right} onClick={() => chain().setTextAlign("right").run()}><AlignRight size={15} /></Tool>
      <Sep />
      <Tool label="Lien" kbd="mod+k" active={st.link} onClick={openLink}><Link2 size={15} /></Tool>
      <Dropdown.Root>
        <Dropdown.Trigger asChild>
          <IconButton label="Couleur du texte" tooltipSide="top" {...keepFocus}><Palette size={15} /></IconButton>
        </Dropdown.Trigger>
        <Dropdown.Portal>
          <Dropdown.Content side="top" align="start" sideOffset={8} onCloseAutoFocus={(e) => { e.preventDefault(); editor.commands.focus(); }} className="anim-fade-in z-[70] grid grid-cols-5 gap-1.5 rounded-xl bg-elevated p-2 shadow-pop">
            {COLORS.map((c) => (
              <Dropdown.Item key={c} aria-label={c} onSelect={() => (c === COLORS[0] ? chain().unsetColor().run() : chain().setColor(c).run())} className="h-6 w-6 cursor-default rounded-full outline-none ring-1 ring-border-strong data-[highlighted]:ring-2 data-[highlighted]:ring-accent" style={{ background: c }} />
            ))}
          </Dropdown.Content>
        </Dropdown.Portal>
      </Dropdown.Root>
      <Tool label="Effacer la mise en forme" onClick={() => chain().unsetAllMarks().clearNodes().unsetTextAlign().run()}><RemoveFormatting size={15} /></Tool>
      <span className="flex-1" />
      <Tool label="Annuler" kbd="mod+z" disabled={!st.canUndo} onClick={() => chain().undo().run()}><Undo2 size={15} /></Tool>
      <Tool label="Rétablir" kbd={isMac ? "shift+mod+z" : "mod+y"} disabled={!st.canRedo} onClick={() => chain().redo().run()}><Redo2 size={15} /></Tool>
    </div>
  );
}

/** Éditeur de message : gras, italique, listes, citation, alignement, couleur, liens. Sortie en HTML. */
export function RichEditor({ html, onChange, focusAtStart }: { html: string; onChange: (html: string) => void; focusAtStart: boolean }) {
  const editor = useEditor({
    extensions: [
      StarterKit.configure({ heading: { levels: [2, 3] }, link: { openOnClick: false, autolink: true, protocols: ["mailto"], HTMLAttributes: { rel: "noopener noreferrer" } } }),
      TextAlign.configure({ types: ["heading", "paragraph"] }),
      TextStyle,
      ThemedColor,
      Placeholder.configure({ placeholder: "Écrivez votre message…" }),
    ],
    content: html,
    autofocus: focusAtStart ? "start" : false,
    onUpdate: ({ editor: e }) => onChange(e.isEmpty ? "" : e.getHTML()),
    editorProps: { attributes: { "aria-label": "Message", spellcheck: "true", class: "selectable" } },
  });
  if (!editor) return null;

  return (
    <>
      <div
        className="composer-editor min-h-0 flex-1 cursor-text overflow-y-auto px-4 py-3"
        onKeyDown={(e) => {
          // ⌘K / Ctrl+K dans l'éditeur : lien (et non la palette de commandes).
          if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") e.stopPropagation();
        }}
        onClick={(e) => { if (e.target === e.currentTarget) editor.commands.focus("end"); }}
      >
        <EditorContent editor={editor} />
      </div>
      <FormatBar editor={editor} />
    </>
  );
}
