import { useEffect, useState } from "react";
import { Inbox, Archive, Star, Send, Bot } from "lucide-react";
import { api, type AppInfo } from "@/lib/api";
import DevPanel from "@/components/DevPanel";

/** Squelette 3 colonnes : navigation | liste des conversations | lecture. (Étape 4 : à développer.) */
export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.appInfo().then(setInfo).catch((e) => setError(String(e)));
  }, []);

  const nav = [
    { icon: Inbox, label: "Boîte de réception" },
    { icon: Star, label: "Suivis" },
    { icon: Send, label: "Envoyés" },
    { icon: Archive, label: "Archives" },
    { icon: Bot, label: "Agent IA (MCP)" },
  ];

  return (
    <div className="grid h-full grid-cols-[220px_460px_1fr] text-sm">
      <aside className="border-r border-border bg-muted p-3 pt-10">
        {nav.map(({ icon: Icon, label }) => (
          <div key={label} className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-background">
            <Icon size={16} /> {label}
          </div>
        ))}
      </aside>
      <section className="border-r border-border text-muted-foreground"><DevPanel /></section>
      <main className="p-4 pt-10">
        <p className="text-muted-foreground">Sélectionnez une conversation.</p>
        <p className="mt-6 text-xs text-muted-foreground">
          {error ? `Backend : ${error}` : info ? `${info.name} v${info.version} — DB : ${info.dbPath}` : "Connexion au backend…"}
        </p>
      </main>
    </div>
  );
}
