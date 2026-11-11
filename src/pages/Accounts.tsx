import { useState } from "react";
import { ArrowRightLeft, CircleCheck, Clock, Gauge, Plus, RefreshCw, Users } from "lucide-react";
import { AccountProfile } from "../types";
import RemoveAccountDialog from "../RemoveAccountDialog";
import { accountName, Avatar, Badge, Button, Card, formatAgo, Menu, Meter, QuotaBar, Segmented, StatCard } from "../ui";
import { isUnknownPlan, pct, t } from "../i18n";

type QuotaView = "both" | "five" | "week";

// The fuller of the two windows decides how much is left.
const fuller = (a: AccountProfile) => Math.max(a.usage.fiveHour?.utilization ?? 0, a.usage.sevenDay?.utilization ?? 0);

const hasData = (a: AccountProfile) => !!(a.usage.fiveHour || a.usage.sevenDay);

/** uuid of the account with the lowest fuller window; needs at least two accounts with data. A tie gives none. */
function emptiestUuid(accounts: AccountProfile[]): string | null {
  const ranked = accounts.filter(hasData).sort((a, b) => fuller(a) - fuller(b));
  return ranked.length >= 2 && fuller(ranked[0]) < fuller(ranked[1]) ? ranked[0].accountUuid : null;
}

function AccountRow({
  acc,
  view,
  busy,
  onSwitch,
  onRename,
  first,
  last,
  onMove,
  onRemove,
  emptiest,
}: {
  acc: AccountProfile;
  emptiest: boolean;
  view: QuotaView;
  busy: boolean;
  onSwitch: () => void;
  onRename: (label: string) => void;
  first: boolean;
  last: boolean;
  onMove: (direction: "up" | "down") => void;
  onRemove: () => void;
}) {
  const { name, hint } = accountName(acc);
  const [editing, setEditing] = useState(false);
  const finishEdit = (value: string | null) => {
    setEditing(false); // Esc passes null: leave without saving
    if (value !== null && value.trim() !== (acc.customLabel ?? "")) onRename(value);
  };
  const { fiveHour, sevenDay, sevenDaySonnet } = acc.usage;
  const hasQuota = fiveHour || sevenDay;
  const rows = [
    view !== "week" && fiveHour && { key: t("acc.rowFive"), title: t("acc.fiveHourLimit"), bars: [{ label: t("acc.allModels"), win: fiveHour }] },
    view !== "five" &&
      sevenDay && {
        key: t("acc.rowWeek"),
        title: t("acc.weeklyLimit"),
        bars: [{ label: t("acc.allModels"), win: sevenDay }, ...(sevenDaySonnet ? [{ label: "Sonnet", win: sevenDaySonnet }] : [])],
      },
  ].filter(Boolean) as { key: string; title: string; bars: { label: string; win: NonNullable<typeof fiveHour> }[] }[];

  return (
    <Card className="flex gap-3 p-4">
      <Avatar name={name} seed={acc.accountUuid} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          {editing ? (
            <input
              autoFocus
              defaultValue={acc.customLabel ?? ""}
              maxLength={40}
              placeholder={t("acc.namePlaceholder")}
              aria-label={t("acc.nameLabel")}
              onBlur={(e) => finishEdit(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
                else if (e.key === "Escape") {
                  e.currentTarget.value = acc.customLabel ?? ""; // a blur on unmount must not save
                  finishEdit(null);
                }
              }}
              className="w-56 rounded-md border border-zinc-300 dark:border-white/15 bg-white dark:bg-[#17181c] px-2 py-0.5 text-[15px] font-semibold text-zinc-900 dark:text-zinc-100 outline-none focus:border-[#D97757] focus:ring-2 focus:ring-[#D97757]/30"
            />
          ) : (
            <h3 className="truncate text-[15px] font-semibold text-zinc-900 dark:text-zinc-100">{name}</h3>
          )}
          {!isUnknownPlan(acc.planType) && <Badge>{acc.planType}</Badge>}
          {emptiest && <Badge tone="green">{t("acc.emptiest")}</Badge>}
        </div>
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
          {hint && <span className="truncate font-mono text-zinc-400">{hint}</span>}
          <Badge tone="green">Desktop</Badge>
          {acc.hasDpapiCredentials && <Badge tone="sky">CLI</Badge>}
          <span className="tabular-nums">{t("count.chats", { n: acc.sessionCount })}</span>
        </div>

        {rows.map((r) => (
          <div key={r.key} className="mt-3 flex items-start gap-3">
            <span className="w-7 shrink-0 pt-px text-xs font-medium text-zinc-500 dark:text-zinc-400" title={r.title}>
              {r.key}
            </span>
            <div className="grid flex-1 grid-cols-[repeat(auto-fill,minmax(170px,210px))] gap-x-6 gap-y-2">
              {r.bars.map((b) => (
                <QuotaBar key={b.label} label={b.label} ariaLabel={`${r.title}, ${b.label}`} win={b.win} />
              ))}
            </div>
          </div>
        ))}
        <div className="mt-2 space-y-0.5 text-[11px] text-zinc-400">
          {!hasQuota && <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("acc.noQuota")}</p>}
          {hasQuota && acc.usage.source === "desktop-history" && <p>{t("acc.fromDesktop", { ago: formatAgo(acc.usage.fetchedAt) })}</p>}
          {!acc.hasDpapiCredentials && <p>{t("acc.cliHint")}</p>}
          {acc.usage.rateLimitNotice && <p className="text-xs text-rose-600 dark:text-rose-400">{acc.usage.rateLimitNotice}</p>}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1 self-center">
        {acc.isActive ? (
          <span className="inline-flex items-center gap-1.5 rounded-md bg-emerald-50 dark:bg-emerald-400/10 px-2.5 py-1.5 text-xs font-medium text-emerald-700 dark:text-emerald-300 ring-1 ring-inset ring-emerald-200 dark:ring-emerald-400/25">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" aria-hidden />
            {t("acc.active")}
          </span>
        ) : (
          <Button onClick={onSwitch} disabled={busy} className="h-8 px-3 text-xs" aria-label={t("acc.switchTo", { name })}>
            <ArrowRightLeft className="h-3.5 w-3.5" aria-hidden />
            {t("acc.switch")}
          </Button>
        )}
        <Menu
          label={t("acc.actionsFor", { name })}
          items={[
            { label: t("acc.rename"), onSelect: () => setEditing(true) },
            { label: t("acc.moveUp"), onSelect: () => onMove("up"), disabled: first },
            { label: t("acc.moveDown"), onSelect: () => onMove("down"), disabled: last },
            {
              label: t("acc.remove"),
              onSelect: onRemove,
              danger: true,
              disabled: acc.isActive || busy,
              title: acc.isActive ? t("acc.removeActive") : undefined,
            },
          ]}
        />
      </div>
    </Card>
  );
}

export default function AccountsPage({
  accounts,
  busy,
  usageBusy,
  selecting,
  onRefresh,
  onAdd,
  onSwitch,
  onRename,
  onMove,
  onRemove,
}: {
  accounts: AccountProfile[];
  busy: boolean;
  usageBusy: boolean;
  selecting: boolean;
  onRefresh: () => void;
  onAdd: () => void;
  onSwitch: (acc: AccountProfile) => void;
  onRename: (acc: AccountProfile, label: string) => void;
  onMove: (acc: AccountProfile, direction: "up" | "down") => void;
  onRemove: (acc: AccountProfile) => void;
}) {
  const [view, setView] = useState<QuotaView>("both");
  const [removing, setRemoving] = useState<AccountProfile | null>(null);
  const active = accounts.find((a) => a.isActive);
  const withData = accounts.filter(hasData);
  const emptiest = emptiestUuid(accounts);
  const limited = accounts.filter((a) => fuller(a) >= 100 || a.usage.rateLimitNotice).length;
  const avgLeft = withData.length
    ? Math.round(withData.reduce((s, a) => s + Math.max(0, 100 - fuller(a)), 0) / withData.length)
    : null;

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-4 gap-3">
        <StatCard label={t("acc.total")} value={accounts.length} icon={Users} tone="sky" />
        <StatCard label={t("acc.open")} value={active ? accountName(active).name : "—"} icon={CircleCheck} tone="green" small />
        <StatCard label={t("acc.limited")} value={limited} icon={Clock} tone="amber" />
        <StatCard label={t("acc.avgLeft")} value={avgLeft === null ? "—" : pct(avgLeft)} icon={Gauge} tone="white">
          {avgLeft !== null && (
            <div className="min-w-0 max-w-[3.5rem] flex-1">
              <Meter pct={100 - avgLeft} fill={avgLeft} label={t("acc.avgLeft")} />
            </div>
          )}
        </StatCard>
      </div>

      <Card>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <p className="min-w-0 text-pretty text-xs leading-relaxed text-zinc-500 dark:text-zinc-400">
            {t("acc.info")} {selecting ? t("acc.infoAsk") : t("acc.infoAll")} {t("acc.infoNew")}
          </p>
          <div className="flex shrink-0 items-center gap-2">
            <Button onClick={onRefresh} disabled={usageBusy}>
              <RefreshCw className={`h-4 w-4 ${usageBusy ? "animate-spin" : ""}`} aria-hidden />
              {t("acc.refresh")}
            </Button>
            <Button variant="primary" onClick={onAdd} disabled={busy}>
              <Plus className="h-4 w-4" aria-hidden />
              {t("acc.add")}
            </Button>
          </div>
        </div>
        <div className="flex items-center justify-between gap-4 border-t border-zinc-100 dark:border-white/[0.06] px-4 py-2.5">
          <span className="text-[13px] font-medium text-zinc-600 dark:text-zinc-400">{t("acc.quotaView")}</span>
          <Segmented
            label={t("acc.quotaViewLabel")}
            value={view}
            onChange={setView}
            options={[
              { value: "both", label: t("acc.viewBoth") },
              { value: "five", label: t("quota.fiveHour") },
              { value: "week", label: t("quota.weekly") },
            ]}
          />
        </div>
      </Card>

      <section className="space-y-3" aria-label={t("acc.list")}>
        {accounts.length === 0 && <Card className="p-6 text-sm text-zinc-500 dark:text-zinc-400">{t("acc.empty")}</Card>}
        {accounts.map((acc, i) => (
          <AccountRow
            key={acc.accountUuid}
            acc={acc}
            emptiest={acc.accountUuid === emptiest}
            view={view}
            busy={busy}
            onSwitch={() => onSwitch(acc)}
            onRename={(l) => onRename(acc, l)}
            first={i === 0}
            last={i === accounts.length - 1}
            onMove={(d) => onMove(acc, d)}
            onRemove={() => setRemoving(acc)}
          />
        ))}
      </section>

      {removing && (
        <RemoveAccountDialog
          name={accountName(removing).name}
          onCancel={() => setRemoving(null)}
          onConfirm={() => {
            setRemoving(null);
            onRemove(removing);
          }}
        />
      )}
    </div>
  );
}
