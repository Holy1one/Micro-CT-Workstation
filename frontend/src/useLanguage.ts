import { useEffect, useSyncExternalStore } from "react";
import { getLanguage, readSavedLanguage, setLanguage, subscribeLanguage, LANGUAGE_KEY } from "./i18n";

/** No locale-dependent keys/remounts: switching languages preserves all UI drafts. */
export function useLanguage() {
  const language = useSyncExternalStore(subscribeLanguage, getLanguage, () => "en" as const);
  useEffect(() => {
    document.documentElement.lang = language;
    document.documentElement.dataset.language = language;
    // The product name is a brand, not localizable interface copy.
    document.title = "Micro-CT Workstation";
  }, [language]);
  useEffect(() => {
    const sync = (event: StorageEvent) => { if (event.key === LANGUAGE_KEY || event.key === null) setLanguage(readSavedLanguage()); };
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, []);
  return [language, setLanguage] as const;
}
