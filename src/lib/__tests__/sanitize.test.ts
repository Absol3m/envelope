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
