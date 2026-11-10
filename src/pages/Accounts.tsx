import { useState } from "react";
import { ArrowRightLeft, CircleCheck, Clock, Gauge, Plus, RefreshCw, Users } from "lucide-react";
import { AccountProfile } from "../types";
import { accountName, Avatar, Badge, Button, Card, formatAgo, Menu, Meter, QuotaBar, Segmented, StatCard } from "../ui";

type QuotaView = "both" | "five" | "week";

// The fuller of the two windows decides how much is left.
const fuller = (a: AccountProfile) => Math.max(a.usage.fiveHour?.utilization ?? 0, a.usage.sevenDay?.utilization ?? 0);

function AccountRow({
  acc,
  view,
  busy,
  onSwitch,
  onRename,
}: {
  acc: AccountProfile;
  view: QuotaView;
  busy: boolean;
  onSwitch: () => void;
  onRename: (label: string) => void;
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
    view !== "week" && fiveHour && { key: "5s", title: "5 saatlik limit", bars: [{ label: "Tüm modeller", win: fiveHour }] },
    view !== "five" &&
      sevenDay && {
        key: "Hft",
        title: "Haftalık limit",
        bars: [{ label: "Tüm modeller", win: sevenDay }, ...(sevenDaySonnet ? [{ label: "Sonnet", win: sevenDaySonnet }] : [])],
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
              placeholder="Hesaba isim ver"
              aria-label="Hesap ismi"
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
          {acc.planType !== "Plan bilinmiyor" && <Badge>{acc.planType}</Badge>}
        </div>
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
          {hint && <span className="truncate font-mono text-zinc-400">{hint}</span>}
          <Badge tone="green">Desktop</Badge>
          {acc.hasDpapiCredentials && <Badge tone="sky">CLI</Badge>}
          <span className="tabular-nums">{acc.sessionCount} sohbet</span>
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
          {!hasQuota && <p className="text-xs text-zinc-500 dark:text-zinc-400">Kota bilgisi yok. Bu hesap Claude Desktop'ta bir kez açılınca görünür.</p>}
          {hasQuota && acc.usage.source === "desktop-history" && (
            <p>Claude Desktop'un kaydından · {formatAgo(acc.usage.fetchedAt)}</p>
          )}
          {!acc.hasDpapiCredentials && <p>Anlık kota için terminalde bu hesapla bir kez giriş yap: claude → /login</p>}
          {acc.usage.rateLimitNotice && <p className="text-xs text-rose-600 dark:text-rose-400">{acc.usage.rateLimitNotice}</p>}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1 self-center">
        {acc.isActive ? (
          <span className="inline-flex items-center gap-1.5 rounded-md bg-emerald-50 dark:bg-emerald-400/10 px-2.5 py-1.5 text-xs font-medium text-emerald-700 dark:text-emerald-300 ring-1 ring-inset ring-emerald-200 dark:ring-emerald-400/25">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" aria-hidden />
            Açık
          </span>
        ) : (
          <Button onClick={onSwitch} disabled={busy} className="h-8 px-3 text-xs" aria-label={`${name} hesabına geç`}>
            <ArrowRightLeft className="h-3.5 w-3.5" aria-hidden />
            Geç
          </Button>
        )}
        <Menu label={`${name} için işlemler`} items={[{ label: "İsim ver", onSelect: () => setEditing(true) }]} />
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
}: {
  accounts: AccountProfile[];
  busy: boolean;
  usageBusy: boolean;
  selecting: boolean;
  onRefresh: () => void;
  onAdd: () => void;
  onSwitch: (acc: AccountProfile) => void;
  onRename: (acc: AccountProfile, label: string) => void;
}) {
  const [view, setView] = useState<QuotaView>("both");
  const active = accounts.find((a) => a.isActive);
  const withData = accounts.filter((a) => a.usage.fiveHour || a.usage.sevenDay);
  const limited = accounts.filter((a) => fuller(a) >= 100 || a.usage.rateLimitNotice).length;
  const avgLeft = withData.length
    ? Math.round(withData.reduce((s, a) => s + Math.max(0, 100 - fuller(a)), 0) / withData.length)
    : null;

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-4 gap-3">
        <StatCard label="Toplam hesap" value={accounts.length} icon={Users} tone="sky" />
        <StatCard label="Açık hesap" value={active ? accountName(active).name : "—"} icon={CircleCheck} tone="green" small />
        <StatCard label="Limite takılan" value={limited} icon={Clock} tone="amber" />
        <StatCard label="Ortalama kalan kota" value={avgLeft === null ? "—" : `%${avgLeft}`} icon={Gauge} tone="white">
          {avgLeft !== null && (
            <div className="min-w-0 max-w-[3.5rem] flex-1">
              <Meter pct={100 - avgLeft} fill={avgLeft} label="Ortalama kalan kota" />
            </div>
          )}
        </StatCard>
      </div>

      <Card>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <p className="min-w-0 text-pretty text-xs leading-relaxed text-zinc-500 dark:text-zinc-400">
            Hesap değiştirince Claude Desktop birkaç saniyeliğine kapanıp açılır.{" "}
            {selecting
              ? "Onayladığın sohbetler diğer hesaplara kendiliğinden aktarılır, yenileri geçişte sorulur;"
              : "Sohbetler tüm hesaplara kendiliğinden aktarılır;"}{" "}
            yeni giriş yapılan hesaplar kendiliğinden kaydedilir.
          </p>
          <div className="flex shrink-0 items-center gap-2">
            <Button onClick={onRefresh} disabled={usageBusy}>
              <RefreshCw className={`h-4 w-4 ${usageBusy ? "animate-spin" : ""}`} aria-hidden />
              Limitleri yenile
            </Button>
            <Button variant="primary" onClick={onAdd} disabled={busy}>
              <Plus className="h-4 w-4" aria-hidden />
              Yeni hesap ekle
            </Button>
          </div>
        </div>
        <div className="flex items-center justify-between gap-4 border-t border-zinc-100 dark:border-white/[0.06] px-4 py-2.5">
          <span className="text-[13px] font-medium text-zinc-600 dark:text-zinc-400">Kota görünümü</span>
          <Segmented
            label="Gösterilecek kota"
            value={view}
            onChange={setView}
            options={[
              { value: "both", label: "İkisi" },
              { value: "five", label: "5 saatlik" },
              { value: "week", label: "Haftalık" },
            ]}
          />
        </div>
      </Card>

      <section className="space-y-3" aria-label="Hesap listesi">
        {accounts.length === 0 && (
          <Card className="p-6 text-sm text-zinc-500 dark:text-zinc-400">
            Henüz hesap yok. Claude Desktop'ta giriş yaptığın hesap burada otomatik görünür.
          </Card>
        )}
        {accounts.map((acc) => (
          <AccountRow
            key={acc.accountUuid}
            acc={acc}
            view={view}
            busy={busy}
            onSwitch={() => onSwitch(acc)}
            onRename={(l) => onRename(acc, l)}
          />
        ))}
      </section>
    </div>
  );
}
