// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { TextStyle } from "@tiptap/extension-text-style";
import { ThemedColor } from "@/components/RichEditor";

/** HTML tel que l'éditeur le garde après l'avoir lu (collage, citation, brouillon). */
const kept = (html: string) => {
  const editor = new Editor({ extensions: [StarterKit, TextStyle, ThemedColor], content: html });
  const out = editor.getHTML();
  editor.destroy();
  return out;
};

describe("couleur de texte dans l'éditeur", () => {
  it("le noir et le blanc d'origine disparaissent, le texte prend la couleur du thème", () => {
    const out = kept('<p><span style="color:#000">noir</span> <span style="color:white">blanc</span> <span style="color:rgb(34,34,34)">gris</span></p>');
    expect(out).toBe("<p><span>noir</span> <span>blanc</span> <span>gris</span></p>"); // spans sans style : sans effet
  });
  it("les vraies couleurs et celles de la palette restent", () => {
    const out = kept('<p><span style="color:#dc2626">rouge</span> <span style="color:#6b6b78">gris palette</span></p>');
    expect(out).toBe('<p><span style="color: rgb(220, 38, 38);">rouge</span> <span style="color: rgb(107, 107, 120);">gris palette</span></p>');
  });
});
