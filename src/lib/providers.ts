import type { OAuthProviderId } from "./api";
import { t } from "./i18n";

/** Étapes lues à l'affichage (langue active) : les consoles Google et Microsoft changent, les textes sont dans les fichiers de langue. */
const steps = (provider: OAuthProviderId, count: number) => Array.from({ length: count }, (_, i) => t(`providers:${provider}.steps.${i + 1}`));

export const OAUTH_INFO: Record<OAuthProviderId, { label: string; needsSecret: boolean; readonly steps: string[] }> = {
  google: {
    label: "Google",
    needsSecret: true,
    get steps() { return steps("google", 4); },
  },
  microsoft: {
    label: "Microsoft",
    needsSecret: false,
    get steps() { return steps("microsoft", 4); },
  },
};
