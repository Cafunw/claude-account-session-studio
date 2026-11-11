import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Download, Loader2, X } from "lucide-react";
import { Button, focusRing } from "./ui";
import { t } from "./i18n";

/** Downloads, installs and restarts; on Windows the installer closes the app itself, so this only returns on error. */
export const installUpdate = () => invoke<void>("install_update");

/** Shown when the background check (every 6 h) finds a new release while the app runs. */
export default function UpdateBanner({
  version,
  onDismiss,
  onError,
}: {
  version: string;
  onDismiss: () => void;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const install = async () => {
    setBusy(true);
    try {
      await installUpdate();
    } catch (e: any) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div role="status" className="mb-6 flex items-center gap-3 rounded-xl border border-[#D97757]/40 dark:border-[#D97757]/35 bg-[#D97757]/[0.06] dark:bg-[#D97757]/10 px-4 py-3">
      <Download className="h-5 w-5 shrink-0 text-[#C2603F] dark:text-[#E89A7E]" aria-hidden />
      <p className="min-w-0 flex-1 text-sm font-medium text-zinc-900 dark:text-zinc-100">{t("update.ready", { version })}</p>
      <Button variant="primary" className="h-8 px-3 text-xs" onClick={install} disabled={busy}>
        {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />}
        {busy ? t("common.updating") : t("common.updateNow")}
      </Button>
      <button
        type="button"
        onClick={onDismiss}
        aria-label={t("update.dismiss")}
        className={`rounded-md p-1 text-zinc-500 hover:bg-zinc-900/5 hover:text-zinc-800 dark:text-zinc-400 dark:hover:bg-white/10 dark:hover:text-zinc-100 ${focusRing}`}
      >
        <X className="h-4 w-4" aria-hidden />
      </button>
    </div>
  );
}
