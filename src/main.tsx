import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { watchSystemAppearance } from "@/lib/theme";
import { useMail } from "@/lib/store";
import { suppressWebviewContextMenu } from "@/lib/platform";
import { applyFont, savedFont } from "@/lib/font";
import { useTranslation } from "react-i18next";
import "@/lib/i18n";
import "./index.css";

suppressWebviewContextMenu(document);
void applyFont(savedFont());
// Thème posé avant le premier rendu (pas d'éclair blanc en mode sombre), puis couleurs du système.
watchSystemAppearance(() => useMail.getState());
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
