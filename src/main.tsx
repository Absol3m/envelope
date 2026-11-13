import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { watchSystemAppearance } from "@/lib/theme";
import { useMail } from "@/lib/store";
import { suppressWebviewContextMenu } from "@/lib/platform";
import { applyFont, savedFont } from "@/lib/font";
import { useTranslation } from "react-i18next";
import { i18next } from "@/lib/i18n";
import { isTauri, pushNativeTexts } from "@/lib/api";
import "./index.css";

suppressWebviewContextMenu(document);
void applyFont(savedFont());
// Thème posé avant le premier rendu (pas d'éclair blanc en mode sombre), puis couleurs du système.
watchSystemAppearance(() => useMail.getState());
// Textes des fenêtres natives (invite de déverrouillage, sélecteurs…) dans la langue de l'interface, dès le démarrage.
if (isTauri()) {
  void pushNativeTexts();
  i18next.on("languageChanged", () => void pushNativeTexts());
}
if (import.meta.env.DEV) void import("./dev").then((m) => m.applyDevParams());

/** Les textes sont lus à l'affichage ; changer de langue remonte l'interface pour que tout (y compris les composants
 *  mémoïsés) soit relu. L'état vit dans les stores : sélection, panneau de réglages et brouillon sont conservés. */
function Root() {
  const { i18n } = useTranslation();
  return <App key={i18n.language} />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
