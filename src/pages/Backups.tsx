import { Archive, DatabaseBackup, RotateCcw } from "lucide-react";
import { BackupManifest } from "../types";
import { Badge, Button, Card, formatDateShort } from "../ui";
import { backupReason, t } from "../i18n";

export default function BackupsPage({
  backups,
  busy,
  onBackup,
  onRestore,
}: {
  backups: BackupManifest[];
  busy: boolean;
  onBackup: () => void;
  onRestore: (id: string) => void;
}) {
  return (
    <div className="space-y-4">
      <Card className="flex items-center justify-between gap-4 px-4 py-3">
        <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("backups.info")}</p>
        <Button variant="primary" onClick={onBackup} disabled={busy}>
          <DatabaseBackup className="h-4 w-4" aria-hidden />
          {t("backups.now")}
        </Button>
      </Card>

      <Card className="overflow-hidden">
        <ul className="divide-y divide-zinc-100 dark:divide-white/[0.06]">
          {backups.length === 0 && <li className="py-8 text-center text-sm text-zinc-500 dark:text-zinc-400">{t("backups.empty")}</li>}
          {backups.map((b) => (
            <li key={b.id} className="flex items-center gap-3 px-4 py-3">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-zinc-100 dark:bg-white/[0.06] text-zinc-500 dark:text-zinc-400" aria-hidden>
                <Archive className="h-4 w-4" />
              </span>
              <div className="min-w-0 flex-1">
                <p className="flex items-center gap-2 truncate text-sm font-medium text-zinc-800 dark:text-zinc-200">
                  <span className="truncate">{backupReason(b.reason)}</span>
                  {b.allVerified && <Badge tone="green">{t("backups.verified")}</Badge>}
                </p>
                <p className="text-xs tabular-nums text-zinc-400">
                  {formatDateShort(b.createdAt)} · {t("count.files", { n: b.totalFiles })}
                </p>
              </div>
              <Button onClick={() => onRestore(b.id)} disabled={busy} className="h-8 px-3 text-xs">
                <RotateCcw className="h-3.5 w-3.5" aria-hidden />
                {t("backups.restore")}
              </Button>
            </li>
          ))}
        </ul>
      </Card>
    </div>
  );
}
