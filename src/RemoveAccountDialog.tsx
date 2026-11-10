import { useEffect, useRef } from "react";
import { Button } from "./ui";

/** Confirms removing an account from the app; same modal look as SwitchDialog. */
export default function RemoveAccountDialog({
  name,
  onConfirm,
  onCancel,
}: {
  name: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    // Destructive: start on "Vazgeç".
    box.current?.querySelector<HTMLElement>("[data-cancel]")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return onCancel();
      if (e.key !== "Tab" || !box.current) return;
      const f = [...box.current.querySelectorAll<HTMLElement>("button:not(:disabled)")];
      const [first, last] = [f[0], f[f.length - 1]];
      const wrapTo = e.shiftKey ? (document.activeElement === first ? last : null) : document.activeElement === last ? first : null;
      if (!wrapTo) return;
      e.preventDefault();
      wrapTo.focus();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      opener?.focus();
    };
    // Mount-only, like SwitchDialog.
  }, []);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-zinc-900/25 p-6 backdrop-blur-[2px] dark:bg-black/50" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div
        ref={box}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="remove-dialog-title"
        aria-describedby="remove-dialog-text"
        className="w-full max-w-md rounded-2xl border border-zinc-200 bg-white shadow-xl dark:border-white/10 dark:bg-[#1b1c21] dark:shadow-black/50"
      >
        <div className="px-5 pt-5">
          <h2 id="remove-dialog-title" className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
            {name} hesabı kaldırılsın mı?
          </h2>
          <div id="remove-dialog-text">
            <p className="mt-3 text-[13px] font-medium text-zinc-700 dark:text-zinc-300">Silinen</p>
            <p className="mt-0.5 text-pretty text-sm leading-relaxed text-zinc-600 dark:text-zinc-400">
              Bu uygulamanın kaydettiği giriş kopyası.
            </p>
            <p className="mt-3 text-[13px] font-medium text-zinc-700 dark:text-zinc-300">Silinmeyen</p>
            <p className="mt-0.5 text-pretty text-sm leading-relaxed text-zinc-600 dark:text-zinc-400">
              Sohbetlerin ve Claude Desktop'taki hesabın silinmez; Claude Desktop'ta tekrar giriş yaparsan hesap geri gelir.
            </p>
          </div>
        </div>
        <div className="mt-5 flex flex-wrap items-center justify-end gap-2 border-t border-zinc-100 px-5 py-4 dark:border-white/[0.06]">
          <Button data-cancel variant="ghost" onClick={onCancel}>
            Vazgeç
          </Button>
          <Button variant="danger" onClick={onConfirm}>
            Kaldır
          </Button>
        </div>
      </div>
    </div>
  );
}
