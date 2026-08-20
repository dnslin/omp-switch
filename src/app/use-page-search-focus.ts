import { useEffect, useRef } from "react";

export function usePageSearchFocus(disabled = false) {
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (disabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "f") return;
      event.preventDefault();
      searchRef.current?.focus();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [disabled]);

  return searchRef;
}
