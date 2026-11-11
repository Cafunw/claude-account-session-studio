import { useEffect, useRef, useState } from "react";
import { PendingChat } from "./types";
import { Button, formatDateShort } from "./ui";
import { t } from "./i18n";

const checkboxClass = "h-4 w-4 shrink-0 cursor-pointer rounded accent-[#D97757]";

/** Asks which chats the target account lacks should be copied before a switch. */
export default function SwitchDialog({
  targetName,
  pending,
  onConfirm,
  onCancel,
}: {
  targetName: string;
  pending: PendingChat[];
  onConfirm: (extraChats: string[]) => void;
  onCancel: () => void;
}) {
  const [picked, setPicked] = useState(() => new Set(pending.map((p) => p.cliSessionId)));
  const box = useRef<HTMLDivElement>(null);
  const allBox = useRef<HTMLInputElement>(null);
  const all = picked.size === pending.length;

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    box.current?.querySelector<HTMLElement>("[data-primary]")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return onCancel();
      if (e.key !== "Tab" || !box.current) return;
      // Keep Tab inside the dialog.
      const f = [...box.current.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)")];
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
    // Mount-only: focus must return to the opener once, on close; the first onCancel closes just as well.
  }, []);

  useEffect(() => {
    if (allBox.current) allBox.current.indeterminate = picked.size > 0 && !all;
  }, [picked, all]);

  const toggle = (id: string) =>
    setPicked((s) => {
      const n = new Set(s);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-zinc-900/25 p-6 backdrop-blur-[2px] dark:bg-black/50" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div
        ref={box}
        role="dialog"
        aria-modal="true"
        aria-labelledby="switch-dialog-title"
        aria-describedby="switch-dialog-text"
        className="flex max-h-[85vh] w-full max-w-lg flex-col rounded-2xl border border-zinc-200 bg-white shadow-xl dark:border-white/10 dark:bg-[#1b1c21] dark:shadow-black/50"
      >
        <div className="px-5 pt-5">
          <h2 id="switch-dialog-title" className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
            {t("switch.title", { name: targetName })}
          </h2>
          <p id="switch-dialog-text" className="mt-1.5 text-pretty text-sm leading-relaxed text-zinc-600 dark:text-zinc-400">
            {t("switch.text", { name: targetName })}
          </p>
          <label className="mt-4 flex cursor-pointer items-center gap-3 border-b border-zinc-100 pb-2.5 text-[13px] font-medium text-zinc-700 dark:border-white/[0.06] dark:text-zinc-300">
            <input
              ref={allBox}
              type="checkbox"
              checked={all}
              onChange={() => setPicked(all ? new Set() : new Set(pending.map((p) => p.cliSessionId)))}
              className={checkboxClass}
            />
            {t("switch.selectAll")}
            <span className="ml-auto text-xs font-normal tabular-nums text-zinc-500 dark:text-zinc-400">
              {picked.size}/{pending.length}
            </span>
          </label>
        </div>
        <ul className="min-h-0 flex-1 divide-y divide-zinc-100 overflow-y-auto px-5 dark:divide-white/[0.06]" aria-label={t("switch.list")}>
          {pending.map((p) => (
            <li key={p.cliSessionId}>
              <label className="flex cursor-pointer items-center gap-3 py-2.5">
                <input type="checkbox" checked={picked.has(p.cliSessionId)} onChange={() => toggle(p.cliSessionId)} className={checkboxClass} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm text-zinc-800 dark:text-zinc-200">{p.title}</span>
                  <span className="block truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400">{p.cwd || t("common.noFolder")}</span>
                </span>
                <span className="shrink-0 text-xs tabular-nums text-zinc-500 dark:text-zinc-400">{formatDateShort(p.lastActivityAt)}</span>
              </label>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center justify-end gap-2 border-t border-zinc-100 px-5 py-4 dark:border-white/[0.06]">
          <Button variant="ghost" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
          <Button onClick={() => onConfirm([])}>{t("switch.skip")}</Button>
          <Button data-primary variant="primary" disabled={picked.size === 0} onClick={() => onConfirm([...picked])}>
            {t("switch.confirm")}
          </Button>
        </div>
      </div>
    </div>
  );
}
