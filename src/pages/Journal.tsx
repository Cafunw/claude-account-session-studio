import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2, RefreshCw } from "lucide-react";
import { JournalEntry } from "../types";
import { Button, Card, Segmented, formatAgo } from "../ui";
import { Key, locale, t } from "../i18n";

const OPS = ["one-click-switch", "session-sync", "backup", "prune", "restore", "format-guard", "remove-account", "update", "auto-capture-account", "auto-sync"] as const;
// Unknown operations show their raw id.
const opLabel = (op: string) => ((OPS as readonly string[]).includes(op) ? t(`journal.op.${op}` as Key) : op);

export default function JournalPage({ notify }: { notify: (ok: boolean, message: string) => void }) {
  const [entries, setEntries] = useState<JournalEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<"all" | "error">("all");

  const load = useCallback(async () => {
    setBusy(true);
    try {
      setEntries(await invoke<JournalEntry[]>("get_journal", { limit: 200 }));
    } catch (e: any) {
      notify(false, String(e));
    } finally {
      setBusy(false);
    }
  }, [notify]);
  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const shown = (entries ?? []).filter((e) => filter === "all" || e.status === "error");

  return (
    <div className="space-y-4">
      <Card className="flex items-center justify-between gap-4 px-4 py-3">
        <Segmented
          label={t("journal.filter")}
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: t("journal.all") },
            { value: "error", label: t("journal.errors") },
          ]}
        />
        <Button onClick={load} disabled={busy}>
          {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <RefreshCw className="h-4 w-4" aria-hidden />}
          {t("journal.refresh")}
        </Button>
      </Card>

      <Card className="overflow-hidden">
        <ul className="divide-y divide-zinc-100 dark:divide-white/[0.06]" aria-busy={busy}>
          {entries && shown.length === 0 && (
            <li className="py-8 text-center text-sm text-zinc-500 dark:text-zinc-400">
              {filter === "error" ? t("journal.noErrors") : t("journal.empty")}
            </li>
          )}
          {shown.map((e, i) => {
            const failed = e.status === "error";
            return (
              <li key={`${e.timestamp}-${i}`} className="flex items-start gap-4 px-4 py-3">
                <time
                  dateTime={e.timestamp}
                  title={new Date(e.timestamp).toLocaleString(locale())}
                  className="w-24 shrink-0 pt-0.5 text-xs tabular-nums text-zinc-500 dark:text-zinc-400"
                >
                  {formatAgo(e.timestamp)}
                </time>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-zinc-800 dark:text-zinc-200">{opLabel(e.operation)}</p>
                  {e.details && <p className="break-words text-xs text-zinc-500 dark:text-zinc-400">{e.details}</p>}
                </div>
                <span
                  className={`inline-flex shrink-0 items-center rounded-md px-1.5 py-[3px] text-[11px] font-medium leading-none ring-1 ring-inset ${
                    failed
                      ? "bg-rose-50 text-rose-700 ring-rose-200 dark:bg-rose-400/10 dark:text-rose-300 dark:ring-rose-400/25"
                      : "bg-emerald-50 text-emerald-700 ring-emerald-200 dark:bg-emerald-400/10 dark:text-emerald-300 dark:ring-emerald-400/25"
                  }`}
                >
                  {failed ? t("journal.error") : t("journal.ok")}
                </span>
              </li>
            );
          })}
        </ul>
      </Card>
    </div>
  );
}
