import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2 } from "lucide-react";
import { AccountProfile, BackupManifest, RestorePreview } from "./types";
import { accountName, Button, formatDateShort } from "./ui";
import { t } from "./i18n";

const checkboxClass = "h-4 w-4 shrink-0 cursor-pointer rounded accent-[#D97757]";

/** Shows what a restore of `backup` would add (read-only preview) and lets the user pick the missing chats. */
export default function RestoreDialog({
  backup,
  accounts,
  onConfirm,
  onCancel,
}: {
  backup: BackupManifest;
  accounts: AccountProfile[];
  onConfirm: (files: string[]) => void;
  onCancel: () => void;
}) {
  const [preview, setPreview] = useState<RestorePreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const box = useRef<HTMLDivElement>(null);
  const allBox = useRef<HTMLInputElement>(null);
  const missing = preview ? preview.files.filter((f) => f.status === "missing") : [];
  const different = preview ? preview.files.filter((f) => f.status === "different").length : 0;
  const identical = preview ? preview.files.filter((f) => f.status === "identical").length : 0;
  const all = missing.length > 0 && picked.size === missing.length;

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    box.current?.focus();
    invoke<RestorePreview>("preview_restore", { backupId: backup.id })
      .then((p) => {
        setPreview(p);
        setPicked(new Set(p.files.filter((f) => f.status === "missing").map((f) => f.relativePath)));
      })
      .catch((e) => setError(String(e)));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return onCancel();
      if (e.key !== "Tab" || !box.current) return;
      // Keep Tab inside the dialog.
      const f = [...box.current.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)")];
      if (f.length === 0) return e.preventDefault();
      const [first, last] = [f[0], f[f.length - 1]];
      const wrapTo = e.shiftKey ? (document.activeElement === first || document.activeElement === box.current ? last : null) : document.activeElement === last ? first : null;
      if (!wrapTo) return;
      e.preventDefault();
      wrapTo.focus();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      opener?.focus();
    };
    // Mount-only, like SwitchDialog: one preview per opening, focus returns to the opener on close.
  }, []);

  // Once loaded (or failed), the primary button (or "Close") takes the focus.
  useEffect(() => {
    if (preview || error) box.current?.querySelector<HTMLElement>("[data-primary]")?.focus();
  }, [preview, error]);

  useEffect(() => {
    if (allBox.current) allBox.current.indeterminate = picked.size > 0 && !all;
  }, [picked, all]);

  const toggle = (path: string) =>
    setPicked((s) => {
      const n = new Set(s);
      n.has(path) ? n.delete(path) : n.add(path);
      return n;
    });

  const owner = (uuid: string) => {
    const acc = accounts.find((a) => a.accountUuid === uuid);
    return acc ? accountName(acc).name : uuid.slice(0, 8);
  };

  const muted = "text-pretty text-sm leading-relaxed text-zinc-600 dark:text-zinc-400";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-zinc-900/25 p-6 backdrop-blur-[2px] dark:bg-black/50" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div
        ref={box}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="restore-dialog-title"
        aria-describedby="restore-dialog-text"
        aria-busy={!preview && !error}
        className="flex max-h-[85vh] w-full max-w-lg flex-col rounded-2xl border border-zinc-200 bg-white shadow-xl outline-none dark:border-white/10 dark:bg-[#1b1c21] dark:shadow-black/50"
      >
        <div className="px-5 pt-5">
          <h2 id="restore-dialog-title" className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
            {t("backups.dialogTitle")}
          </h2>
          <p id="restore-dialog-text" className={`mt-1.5 ${muted}`}>
            {!preview && !error && (
              <span className="flex items-center gap-2" role="status">
                <Loader2 className="h-4 w-4 animate-spin text-[#D97757]" aria-hidden />
                {t("progress.verifyingBackup")}
              </span>
            )}
            {error && <span className="text-rose-600 dark:text-rose-400">{error}</span>}
            {preview && (missing.length > 0 ? t("backups.dialogText", { date: formatDateShort(backup.createdAt) }) : t("backups.nothingMissing"))}
          </p>
          {missing.length > 0 && (
            <label className="mt-4 flex cursor-pointer items-center gap-3 border-b border-zinc-100 pb-2.5 text-[13px] font-medium text-zinc-700 dark:border-white/[0.06] dark:text-zinc-300">
              <input
                ref={allBox}
                type="checkbox"
                checked={all}
                onChange={() => setPicked(all ? new Set() : new Set(missing.map((f) => f.relativePath)))}
                className={checkboxClass}
              />
              {t("switch.selectAll")}
              <span className="ml-auto text-xs font-normal tabular-nums text-zinc-500 dark:text-zinc-400">
                {picked.size}/{missing.length}
              </span>
            </label>
          )}
        </div>
        {missing.length > 0 && (
          <ul className="min-h-0 flex-1 divide-y divide-zinc-100 overflow-y-auto px-5 dark:divide-white/[0.06]" aria-label={t("backups.list")}>
            {missing.map((f) => (
              <li key={f.relativePath}>
                <label className="flex cursor-pointer items-center gap-3 py-2.5">
                  <input type="checkbox" checked={picked.has(f.relativePath)} onChange={() => toggle(f.relativePath)} className={checkboxClass} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm text-zinc-800 dark:text-zinc-200">{f.title}</span>
                    <span className="block truncate text-[11px] text-zinc-500 dark:text-zinc-400">{owner(f.accountUuid)}</span>
                  </span>
                  <span className="shrink-0 text-xs tabular-nums text-zinc-500 dark:text-zinc-400">{formatDateShort(f.lastActivityAt)}</span>
                </label>
              </li>
            ))}
          </ul>
        )}
        {preview && (different > 0 || identical > 0) && (
          <div className="space-y-0.5 px-5 pt-3 text-xs text-zinc-500 dark:text-zinc-400">
            {different > 0 && <p>{t("backups.different", { n: different })}</p>}
            {identical > 0 && <p>{t("backups.identical", { n: identical })}</p>}
          </div>
        )}
        <div className="mt-4 flex flex-wrap items-center justify-end gap-2 border-t border-zinc-100 px-5 py-4 dark:border-white/[0.06]">
          {missing.length > 0 ? (
            <>
              <Button variant="ghost" onClick={onCancel}>
                {t("common.cancel")}
              </Button>
              <Button data-primary variant="primary" disabled={picked.size === 0} onClick={() => onConfirm([...picked])}>
                {t("backups.confirm", { n: picked.size })}
              </Button>
            </>
          ) : (
            <Button data-primary onClick={onCancel}>
              {t("backups.close")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
