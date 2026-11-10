import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2, RefreshCw } from "lucide-react";
import { JournalEntry } from "../types";
import { Button, Card, Segmented, formatAgo } from "../ui";

const LABELS: Record<string, string> = {
  "one-click-switch": "Hesap geçişi",
  "session-sync": "Sohbet aktarımı",
  backup: "Yedek",
  prune: "Yedek temizliği",
  restore: "Geri yükleme",
  "format-guard": "Biçim koruması",
  "remove-account": "Hesap kaldırma",
  update: "Güncelleme",
  "auto-capture-account": "Yeni hesap kaydı",
  "auto-sync": "Otomatik aktarım",
};

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
          label="Kayıt filtresi"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "Tümü" },
            { value: "error", label: "Hatalar" },
          ]}
        />
        <Button onClick={load} disabled={busy}>
          {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <RefreshCw className="h-4 w-4" aria-hidden />}
          Yenile
        </Button>
      </Card>

      <Card className="overflow-hidden">
        <ul className="divide-y divide-zinc-100 dark:divide-white/[0.06]" aria-busy={busy}>
          {entries && shown.length === 0 && (
            <li className="py-8 text-center text-sm text-zinc-500 dark:text-zinc-400">
              {filter === "error" ? "Hata kaydı yok." : "Henüz kayıt yok."}
            </li>
          )}
          {shown.map((e, i) => {
            const failed = e.status === "error";
            return (
              <li key={`${e.timestamp}-${i}`} className="flex items-start gap-4 px-4 py-3">
                <time
                  dateTime={e.timestamp}
                  title={new Date(e.timestamp).toLocaleString("tr-TR")}
                  className="w-24 shrink-0 pt-0.5 text-xs tabular-nums text-zinc-500 dark:text-zinc-400"
                >
                  {formatAgo(e.timestamp)}
                </time>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-zinc-800 dark:text-zinc-200">{LABELS[e.operation] ?? e.operation}</p>
                  {e.details && <p className="break-words text-xs text-zinc-500 dark:text-zinc-400">{e.details}</p>}
                </div>
                <span
                  className={`inline-flex shrink-0 items-center rounded-md px-1.5 py-[3px] text-[11px] font-medium leading-none ring-1 ring-inset ${
                    failed
                      ? "bg-rose-50 text-rose-700 ring-rose-200 dark:bg-rose-400/10 dark:text-rose-300 dark:ring-rose-400/25"
                      : "bg-emerald-50 text-emerald-700 ring-emerald-200 dark:bg-emerald-400/10 dark:text-emerald-300 dark:ring-emerald-400/25"
                  }`}
                >
                  {failed ? "Hata" : "Tamam"}
                </span>
              </li>
            );
          })}
        </ul>
      </Card>
    </div>
  );
}
