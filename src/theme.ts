import { useEffect, useState } from "react";

export type Theme = "system" | "light" | "dark";

const KEY = "theme";
const media = () => matchMedia("(prefers-color-scheme: dark)");

function readTheme(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    if (t === "light" || t === "dark") return t;
  } catch {}
  return "system";
}

/** Theme preference kept in localStorage; "system" follows the OS live. index.html applies it before first paint. */
export function useTheme(): [Theme, (t: Theme) => void] {
  const [theme, setTheme] = useState<Theme>(readTheme);

  useEffect(() => {
    try {
      if (theme === "system") localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, theme);
    } catch {}
    const apply = () =>
      document.documentElement.classList.toggle("dark", theme === "dark" || (theme === "system" && media().matches));
    apply();
    if (theme !== "system") return;
    const m = media();
    m.addEventListener("change", apply);
    return () => m.removeEventListener("change", apply);
  }, [theme]);

  return [theme, setTheme];
}
