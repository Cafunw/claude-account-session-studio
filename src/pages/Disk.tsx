import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FolderOpen, HardDrive, RefreshCw } from "lucide-react";
import { DiskFolderKind, DiskSize, DiskUsage } from "../types";
import { Button, Card, formatBytes, formatDateShort } from "../ui";
import { locale, t } from "../i18n";

const MUTED = "text-xs text-zinc-500 dark:text-zinc-400";
// One magnitude series, the same blue as the Spending page bars.
const BAR = "bg-[#2a78d6] dark:bg-[#3987e5]";
const folderName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() || path;

/** Read-only view of what the Claude data takes on disk. Nothing here changes a file. */
export default function DiskPage({ notify }: { notify: (ok: boolean, message: string) => void }) {
  const [usage, setUsage] = useState<DiskUsage | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setUsage(await invoke<DiskUsage>("get_disk_usage"));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => void load(), [load]);

  const reveal = (kind: DiskFolderKind) => invoke("reveal_disk_folder", { kind }).catch((e) => notify(false, String(e)));

  const sections: { kind: DiskFolderKind; label: string; size: DiskSize | null; sub: string }[] = usage
    ? [
        { kind: "transcripts", label: t("disk.transcripts"), size: usage.transcripts, sub: t("count.files", { n: usage.transcripts.files }) },
        {
          kind: "backups",
          label: t("disk.backups"),
          size: usage.backups.total,
          sub: t("disk.backupSplit", {
            full: `${usage.backups.full.count} (${formatBytes(usage.backups.full.bytes)})`,
            live: `${usage.backups.liveRefresh.count} (${formatBytes(usage.backups.liveRefresh.bytes)})`,
          }),
        },
        { kind: "pointers", label: t("disk.pointers"), size: usage.pointers, sub: t("count.files", { n: usage.pointers.files }) },
        { kind: "app", label: t("disk.app"), size: usage.app, sub: t("count.files", { n: usage.app.files }) },
      ]
    : [];
  const total = sections.reduce((s, x) => s + (x.size?.bytes ?? 0), 0);
  const max = usage?.projects[0]?.size.bytes ?? 0;

  return (
    <div className="space-y-4">
      <Card className="flex items-center justify-between gap-4 px-4 py-3">
        <p className={MUTED}>{t("disk.info")}</p>
        <Button className="h-8" onClick={() => void load()} disabled={loading}>
          <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} aria-hidden />
          {t("disk.refresh")}
        </Button>
      </Card>

      {error && (
        <p role="alert" className="text-sm text-rose-700 dark:text-rose-300">
          {error}
        </p>
      )}

      {!usage ? (
        loading && (
          <div className="space-y-4" aria-busy="true" aria-label={t("disk.loading")}>
            <div className="h-40 animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
            <div className="h-64 animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
          </div>
        )
      ) : (
        <div className={`space-y-4 transition-opacity ${loading ? "opacity-60" : ""}`} aria-busy={loading}>
          <Card className="overflow-hidden">
            <div className="flex items-baseline justify-between gap-3 border-b border-zinc-100 px-4 py-3 dark:border-white/[0.06]">
              <h2 className="text-sm font-medium text-zinc-900 dark:text-zinc-100">{t("disk.total")}</h2>
              <p className="text-2xl font-semibold tabular-nums text-zinc-900 dark:text-zinc-100">{formatBytes(total)}</p>
            </div>
            <ul className="divide-y divide-zinc-100 dark:divide-white/[0.06]">
              {sections.map((s) => (
                <li key={s.kind} className="flex items-center gap-3 px-4 py-2.5">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm text-zinc-800 dark:text-zinc-200">{s.label}</p>
                    <p className={`truncate tabular-nums ${MUTED}`}>{s.sub}</p>
                  </div>
                  <p className="shrink-0 text-sm font-medium tabular-nums text-zinc-900 dark:text-zinc-100">{formatBytes(s.size?.bytes ?? 0)}</p>
                  <Button variant="ghost" className="h-8 px-2 text-xs" onClick={() => void reveal(s.kind)} aria-label={t("disk.showAria", { name: s.label })}>
                    <FolderOpen className="h-4 w-4" aria-hidden />
                    {t("disk.show")}
                  </Button>
                </li>
              ))}
            </ul>
          </Card>

          <Card className="overflow-hidden">
            <h2 className="border-b border-zinc-100 px-4 py-3 text-sm font-medium text-zinc-900 dark:border-white/[0.06] dark:text-zinc-100">{t("disk.projects")}</h2>
            {usage.projects.length === 0 ? (
              <div className="flex flex-col items-center justify-center px-4 py-10 text-center">
                <HardDrive className="mb-2 h-6 w-6 text-zinc-400" aria-hidden />
                <p className="text-sm text-zinc-600 dark:text-zinc-300">{t("disk.empty")}</p>
              </div>
            ) : (
              <table className="w-full table-fixed text-sm">
                <thead className={MUTED}>
                  <tr className="text-left">
                    <th scope="col" className="px-4 py-2 font-normal">{t("disk.colProject")}</th>
                    <th scope="col" className="w-48 px-2 py-2 font-normal">{t("disk.colSize")}</th>
                    <th scope="col" className="w-20 px-2 py-2 text-right font-normal">{t("disk.colFiles")}</th>
                    <th scope="col" className="w-36 px-4 py-2 text-right font-normal">{t("disk.colLast")}</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-zinc-100 dark:divide-white/[0.06]">
                  {usage.projects.map((p) => (
                    <tr key={p.folder}>
                      <td className="px-4 py-2" title={p.path}>
                        <p className="truncate text-zinc-800 dark:text-zinc-200">{folderName(p.path)}</p>
                        <p className={`truncate ${MUTED}`}>{p.path}</p>
                      </td>
                      <td className="px-2 py-2">
                        <div className="flex items-center gap-2">
                          <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-zinc-100 dark:bg-white/[0.06]" aria-hidden>
                            <div className={`h-full rounded-full ${BAR}`} style={{ width: `${max > 0 ? Math.max(1, (p.size.bytes / max) * 100) : 0}%` }} />
                          </div>
                          <span className="w-16 shrink-0 text-right tabular-nums text-zinc-900 dark:text-zinc-100">{formatBytes(p.size.bytes)}</span>
                        </div>
                      </td>
                      <td className="px-2 py-2 text-right tabular-nums text-zinc-600 dark:text-zinc-300">{p.size.files.toLocaleString(locale())}</td>
                      <td className={`px-4 py-2 text-right tabular-nums ${MUTED}`}>{p.size.lastModified ? formatDateShort(p.size.lastModified) : "—"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Card>
        </div>
      )}
    </div>
  );
}
