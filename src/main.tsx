import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, loadTheme } from "@/lib/theme";
import "./index.css";

// Thème posé avant le premier rendu : pas d'éclair blanc au démarrage en mode sombre.
applyTheme(loadTheme());
if (import.meta.env.DEV) void import("./dev").then((m) => m.applyDevParams());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
