import { describe, expect, it } from "vitest";
import { keys, shortcut } from "../platform";

describe("platform", () => {
  it("libellés de raccourcis selon le système", () => {
    expect(keys("mod+k", true)).toEqual(["⌘", "K"]);
    expect(keys("mod+k", false)).toEqual(["Ctrl", "K"]);
    expect(shortcut("shift+mod+z", true)).toBe("⇧⌘Z");
    expect(shortcut("mod+shift+z", false)).toBe("Ctrl+Maj+Z");
    expect(shortcut("mod+enter", false)).toBe("Ctrl+Entrée");
    expect(shortcut("mod+,", true)).toBe("⌘,");
  });

  it("une touche seule reste telle quelle", () => {
    expect(shortcut("e", false)).toBe("E");
    expect(shortcut("Échap", false)).toBe("Échap");
  });
});
