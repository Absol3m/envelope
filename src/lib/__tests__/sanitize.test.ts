// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { prepareHtml } from "../sanitize";

const body = (srcDoc: string) => new DOMParser().parseFromString(srcDoc, "text/html").body;

describe("assainissement des e-mails HTML", () => {
  it("retire scripts, gestionnaires d'événements, formulaires et objets", () => {
    const { srcDoc } = prepareHtml(
      `<p onclick="steal()">Salut</p><script>alert(1)</script><img src="data:image/gif;base64,R0lGOD" onerror="steal()"><form action="https://evil.test"><input name="pw"></form><iframe src="https://evil.test"></iframe><object data="x"></object>`,
      { allowRemote: false },
    );
    const b = body(srcDoc);
    expect(b.querySelector("script,form,input,iframe,object")).toBeNull();
    expect(srcDoc).not.toMatch(/onclick|onerror|steal/);
    expect(b.textContent).toContain("Salut");
  });

  it("neutralise les images distantes et les compte", () => {
    const html = `<img src="https://track.test/pixel.gif"><img src="//cdn.test/a.png"><img src="data:image/png;base64,AAAA">`;
    const blocked = prepareHtml(html, { allowRemote: false });
    expect(blocked.blocked).toBe(2);
    expect(blocked.srcDoc).not.toMatch(/\ssrc="(https?:|\/\/)/); // aucun `src` distant ne subsiste (data-blocked-src n'est pas chargé)
    expect(blocked.srcDoc).toContain("data-blocked-src");
    expect(blocked.srcDoc).toContain("img-src data:;");

    const allowed = prepareHtml(html, { allowRemote: true });
    expect(allowed.blocked).toBe(0);
    expect(allowed.srcDoc).toContain("https://track.test/pixel.gif");
    expect(allowed.srcDoc).toContain("img-src data: https: http:");
  });

  it("l'état d'un appel ne fuit pas dans le suivant", () => {
    prepareHtml(`<img src="https://a.test/x.png">`, { allowRemote: false });
    expect(prepareHtml(`<p>rien</p>`, { allowRemote: false }).blocked).toBe(0);
  });

  it("CSP stricte : aucun chargement réseau hors images autorisées", () => {
    const { srcDoc } = prepareHtml("<p>x</p>", { allowRemote: false });
    expect(srcDoc).toContain("default-src 'none'");
    expect(srcDoc).not.toMatch(/script-src|connect-src/);
  });

  it("remplace cid: par les images inline, sans toucher aux inconnues", () => {
    const { srcDoc, blocked } = prepareHtml(`<img src="cid:logo@x"><img src="cid:autre">`, { allowRemote: false, cid: { "logo@x": "data:image/png;base64,QUJD" } });
    expect(srcDoc).toContain("data:image/png;base64,QUJD");
    expect(blocked).toBe(0);
  });

  it("liens : nouvel onglet sans opener", () => {
    const a = body(prepareHtml(`<a href="https://x.test">lien</a>`, { allowRemote: false }).srcDoc).querySelector("a")!;
    expect(a.getAttribute("target")).toBe("_blank");
    expect(a.getAttribute("rel")).toBe("noopener noreferrer");
  });
});

describe("pixels espions et liens", () => {
  const pixels = `<img src="https://t.test/a.gif" width="1" height="1"><img src="https://t.test/b.gif" style="display: none"><img src="https://t.test/c.gif" height="0"><img src="https://cdn.test/photo.jpg" width="600">`;
  it("retire les images d'un pixel ou masquées, même quand les images distantes sont autorisées", () => {
    const allowed = prepareHtml(pixels, { allowRemote: true });
    expect(allowed.trackers).toBe(3);
    expect(allowed.srcDoc).toContain("https://cdn.test/photo.jpg");
    expect(allowed.srcDoc).not.toMatch(/\ssrc="https:\/\/t\.test/);
    const blocked = prepareHtml(pixels, { allowRemote: false });
    expect([blocked.trackers, blocked.blocked]).toEqual([3, 1]); // la photo reste bloquée tant qu'on ne l'autorise pas
  });
  it("une image locale d'un pixel n'est pas un traqueur", () => {
    expect(prepareHtml(`<img src="data:image/gif;base64,R0lGOD" width="1" height="1">`, { allowRemote: true }).trackers).toBe(0);
  });
  it("affiche la destination des liens au survol, même si l'expéditeur y a mis un autre titre", () => {
    const b = body(prepareHtml(`<a href="https://evil.test/x">paypal.com</a><a href="https://a.test" title="https://banque.fr">a</a>`, { allowRemote: false }).srcDoc);
    const [a1, a2] = Array.from(b.querySelectorAll("a"));
    expect(a1.getAttribute("title")).toBe("https://evil.test/x");
    expect(a2.getAttribute("title")).toBe("https://a.test");
  });
});
