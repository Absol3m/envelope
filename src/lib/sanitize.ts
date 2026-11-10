import DOMPurify from "dompurify";

/* Rendu des e-mails HTML : trois barrières indépendantes.
   1. DOMPurify retire scripts, formulaires, objets, gestionnaires d'événements.
   2. Les images distantes sont neutralisées (pixels espions) tant que l'utilisateur ne les autorise pas.
   3. Le résultat vit dans une iframe `sandbox` (sans scripts) avec une CSP qui interdit tout chargement
      réseau hors images explicitement autorisées. */

const ctx = { allowRemote: false, blocked: 0 };

DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node instanceof HTMLImageElement) {
    const src = node.getAttribute("src") ?? "";
    if (/^(https?:)?\/\//i.test(src) && !ctx.allowRemote) {
      node.setAttribute("data-blocked-src", src);
      node.removeAttribute("src");
      ctx.blocked++;
    }
    node.removeAttribute("srcset");
  }
  if (node.tagName === "A") {
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  }
});

const FORBID_TAGS = ["script", "form", "input", "button", "select", "textarea", "link", "meta", "base", "iframe", "object", "embed", "frame", "frameset"];

const BASE_CSS = `
html{color-scheme:light}
body{margin:0;padding:18px 22px;font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Inter,Helvetica,Arial,sans-serif;color:#1c1c21;background:#fff;overflow-wrap:anywhere}
img{max-width:100%;height:auto}
a{color:#4747c9}
pre{white-space:pre-wrap}
blockquote{margin:0 0 0 2px;padding-left:12px;border-left:3px solid #e3e3ea;color:#666}
img[data-blocked-src]{display:inline-block;min-height:40px;min-width:40px;background:#f1f1f5;border-radius:6px}
`;

export interface Prepared {
  srcDoc: string;
  /** Nombre d'images distantes neutralisées (0 si autorisées ou absentes). */
  blocked: number;
}

/** `cid` : images « inline » du message (identifiant de contenu → data URL). */
export function prepareHtml(html: string, opts: { allowRemote: boolean; cid?: Record<string, string> }): Prepared {
  const withCid = opts.cid ? html.replace(/cid:([^"'\s>)]+)/gi, (m, id: string) => opts.cid?.[id] ?? m) : html;
  ctx.allowRemote = opts.allowRemote;
  ctx.blocked = 0;
  const clean = DOMPurify.sanitize(withCid, { FORBID_TAGS, FORBID_ATTR: ["srcset", "ping"] });
  const img = opts.allowRemote ? "img-src data: https: http:" : "img-src data:";
  const csp = `default-src 'none'; ${img}; style-src 'unsafe-inline'; font-src data:`;
  return {
    srcDoc: `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><style>${BASE_CSS}</style></head><body>${clean}</body></html>`,
    blocked: ctx.blocked,
  };
}

/** Fragment de mail reçu à citer dans une réponse : mêmes barrières que l'affichage, sans images ni styles. */
export function sanitizeFragment(html: string): string {
  return DOMPurify.sanitize(html, { FORBID_TAGS: [...FORBID_TAGS, "img", "style", "head"], FORBID_ATTR: ["srcset", "ping", "style"] });
}

const TRUSTED_KEY = "envelope.trustedSenders";
let trusted: Set<string> | null = null;

/** Expéditeurs dont les images distantes sont autorisées (lus une fois, puis gardés en mémoire). */
export function trustedSenders(): Set<string> {
  if (!trusted) {
    try {
      trusted = new Set(JSON.parse(localStorage.getItem(TRUSTED_KEY) ?? "[]") as string[]);
    } catch {
      trusted = new Set();
    }
  }
  return trusted;
}

export function trustSender(address: string) {
  const set = trustedSenders().add(address.toLowerCase());
  try {
    localStorage.setItem(TRUSTED_KEY, JSON.stringify([...set]));
  } catch {
    /* non mémorisé */
  }
}
