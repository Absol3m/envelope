/** Préférences locales (`localStorage`, clés `envelope.*`) ; indisponible = valeur par défaut. */
export const pref = {
  get<T extends string>(key: string, fallback: T): T {
    try {
      return (localStorage.getItem(`envelope.${key}`) as T | null) ?? fallback;
    } catch {
      return fallback;
    }
  },
  set(key: string, value: string) {
    try {
      localStorage.setItem(`envelope.${key}`, value);
    } catch {
      /* non mémorisé */
    }
  },
};
