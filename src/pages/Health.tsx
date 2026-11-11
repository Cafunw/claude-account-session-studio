import { CircleAlert, CircleCheck, Loader2, Stethoscope } from "lucide-react";
import { HealthReport } from "../types";
import { Button, Card } from "../ui";
import { backupReason, t } from "../i18n";

export default function HealthPage({
  health,
  busy,
  onCheck,
}: {
  health: HealthReport | null;
  busy: boolean;
  onCheck: () => void;
}) {
  return (
    <div className="space-y-4">
      <Card className="flex items-center justify-between gap-4 px-4 py-3">
        <p className="flex items-center gap-2 text-sm text-zinc-600 dark:text-zinc-400" role="status">
          {busy ? (
            <Loader2 className="h-4 w-4 animate-spin text-zinc-400" aria-hidden />
          ) : health && health.items.length === 0 ? (
            <CircleCheck className="h-4 w-4 text-emerald-600 dark:text-emerald-400" aria-hidden />
          ) : health ? (
            <CircleAlert className="h-4 w-4 text-rose-500" aria-hidden />
          ) : null}
          {busy
            ? t("health.checking")
            : !health
              ? t("health.idle")
              : health.items.length === 0
                ? t("health.ok")
                : t("health.found", { n: health.items.length })}
          {health && !busy && (
            <span className="text-xs text-zinc-400">
              {t("health.checked", { chats: t("count.chats", { n: health.checkedChats }), backups: t("health.backups", { n: health.checkedBackups }) })}
            </span>
          )}
        </p>
        <Button variant="primary" onClick={onCheck} disabled={busy}>
          <Stethoscope className="h-4 w-4" aria-hidden />
          {busy ? t("health.checking") : t("health.check")}
        </Button>
      </Card>

      {health && health.items.length > 0 && (
        <Card className="overflow-hidden">
          <ul className="divide-y divide-zinc-100 dark:divide-white/[0.06]">
            {health.items.map((it, i) => (
              <li key={i} className="px-4 py-3">
                <p className="truncate text-sm font-medium text-zinc-800 dark:text-zinc-200">
                  {it.kind === "backup" ? backupReason(it.owner) : it.owner} · {it.subject}
                </p>
                <p className="text-xs text-rose-600 dark:text-rose-400">{it.problem}</p>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}
