import type { OAuthProviderId } from "./api";

export const OAUTH_INFO: Record<OAuthProviderId, { label: string; needsSecret: boolean; steps: string[] }> = {
  google: {
    label: "Google",
    needsSecret: true,
    steps: [
      "Sur console.cloud.google.com, crée un projet (nom libre).",
      "API et services › Bibliothèque : active « Gmail API ».",
      "Écran de consentement OAuth : type « Externe », ajoute ton adresse comme utilisateur test, puis « Publier l'application » (sinon l'autorisation expire au bout de 7 jours).",
      "Identifiants › Créer des identifiants › ID client OAuth › « Application de bureau », puis copie l'ID et le code secret ci-dessous.",
    ],
  },
  microsoft: {
    label: "Microsoft",
    needsSecret: false,
    steps: [
      "Sur portal.azure.com › Microsoft Entra ID › Inscriptions d'applications, crée une application (comptes personnels inclus).",
      "Authentification › Ajouter une plateforme › « Applications mobiles et de bureau », URI de redirection http://127.0.0.1 ; active « Autoriser les flux clients publics ».",
      "Autorisations d'API : ajoute IMAP.AccessAsUser.All, SMTP.Send et offline_access (Office 365 Exchange Online).",
      "Copie l'« ID d'application (client) » ci-dessous.",
    ],
  },
};
