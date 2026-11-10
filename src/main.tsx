import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { watchSystemAppearance } from "@/lib/theme";
import { useMail } from "@/lib/store";
import { suppressWebviewContextMenu } from "@/lib/platform";
import "./index.css";

suppressWebviewContextMenu(document);
// Thème posé avant le premier rendu (pas d'éclair blanc en mode sombre), puis couleurs du système.
watchSystemAppearance(() => useMail.getState());
if (import.meta.env.DEV) void import("./dev").then((m) => m.applyDevParams());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
