import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ArrowDown, ArrowUp, ChevronDown, Coins, Download, FolderOpen, MessagesSquare, Search, X } from "lucide-react";
import { Amount, AutomationSettings, BudgetSpent, FullDashboardPayload, SpendingRange, SpendingReport, TokenSums } from "../types";
import { Button, Card, focusRing, formatDateShort, Segmented, StatCard } from "../ui";
import { locale, t } from "../i18n";

type Metric = "tokens" | "usd";
type Report = SpendingReport;

const TOP = 15;
// Formatters follow the current language, so they are built per call (cheap at this page's scale).
const full = (n: number) => n.toLocaleString(locale());
const compact = (n: number) => new Intl.NumberFormat(locale(), { notation: "compact", maximumFractionDigits: 1 }).format(n);
const usd = (n: number, digits = n < 10 ? 2 : 0) =>
  new Intl.NumberFormat(locale(), { style: "currency", currency: "USD", ...(n >= 10_000 ? { notation: "compact", maximumFractionDigits: 1 } : { maximumFractionDigits: digits }) }).format(n);
const usdFull = (n: number) => new Intl.NumberFormat(locale(), { style: "currency", currency: "USD", maximumFractionDigits: 2 }).format(n);
const fmtMetric = (m: Metric, v: number) => (m === "usd" ? usd(v) : compact(v));
const fullMetric = (m: Metric, v: number) => (m === "usd" ? usdFull(v) : t("spend.tokens", { n: full(v) }));
const amount = (m: Metric, a: Amount | undefined) => (a ? (m === "usd" ? a.cost : a.work) : 0);
const pct = (n: number, of: number) =>
  t("fmt.pct", { n: of > 0 ? ((n / of) * 100).toLocaleString(locale(), { maximumFractionDigits: n / of < 0.1 ? 1 : 0 }) : 0 });
const folderName = (cwd: string) => cwd.split(/[\\/]/).filter(Boolean).pop() || t("spend.unknownFolder");
const dayLabel = (ms: number, weekday = true) =>
  new Date(ms).toLocaleDateString(locale(), { day: "numeric", month: "short", ...(weekday ? { weekday: "short" } : {}) });
const MUTED = "text-xs text-zinc-500 dark:text-zinc-400";

// One magnitude series: categorical slot 1 of the dataviz reference palette (light / dark).
const BAR = "bg-[#2a78d6] dark:bg-[#3987e5]";
// Each model family keeps one fixed slot (color follows the family, never its rank). Validated with
// the dataviz script as adjacent pairs in this stack order, light and dark; slots 3-5 are under 3:1 on
// the light surface, so the legend, tooltips and tables carry every value too.
const FAMILIES = ["opus", "sonnet", "haiku", "fable", "mythos", "other"] as const;
const FAMILY_FILL: Record<string, string> = {
  opus: "fill-[#2a78d6] dark:fill-[#3987e5]",
  sonnet: "fill-[#eb6834] dark:fill-[#d95926]",
  haiku: "fill-[#1baf7a] dark:fill-[#199e70]",
  fable: "fill-[#eda100] dark:fill-[#c98500]",
  mythos: "fill-[#e87ba4] dark:fill-[#d55181]",
  other: "fill-zinc-400 dark:fill-zinc-500",
};
const FAMILY_BG: Record<string, string> = {
  opus: "bg-[#2a78d6] dark:bg-[#3987e5]",
  sonnet: "bg-[#eb6834] dark:bg-[#d95926]",
  haiku: "bg-[#1baf7a] dark:bg-[#199e70]",
  fable: "bg-[#eda100] dark:bg-[#c98500]",
  mythos: "bg-[#e87ba4] dark:bg-[#d55181]",
  other: "bg-zinc-400 dark:bg-zinc-500",
};
const familyName = (f: string) => (f === "other" ? t("spend.other") : f[0].toUpperCase() + f.slice(1));

export default function SpendingPage({
  saveFile,
  automation,
  onDashboard,
}: {
  saveFile: (save: () => Promise<string>) => void;
  automation: AutomationSettings;
  onDashboard: (d: FullDashboardPayload) => void;
}) {
  const [range, setRange] = useState<SpendingRange>("7");
  const [metric, setMetric] = useState<Metric>("tokens");
  const [report, setReport] = useState<Report | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Files read so far by the scan behind the pending request (or the startup warm-up it waits for).
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [filter, setFilter] = useState("");
  const [project, setProject] = useState<string | null>(null);

  useEffect(() => {
    const off = listen<{ done: number; total: number }>("spending-progress", (e) => setProgress(e.payload));
    return () => void off.then((f) => f());
  }, []);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setProgress(null);
    invoke<Report>("get_spending", { range })
      .then((r) => live && (setReport(r), setError(null)))
      .catch((e) => live && setError(String(e)))
      .finally(() => live && (setLoading(false), setProgress(null)));
    return () => {
      live = false;
    };
  }, [range]);

  const ranges: { value: SpendingRange; label: string }[] = [
    { value: "today", label: t("spend.range.today") },
    ...(["7", "30", "90"] as const).map((n) => ({ value: n, label: t("fmt.days", { n: Number(n) }) })),
    { value: "all", label: t("spend.range.all") },
  ];

  return (
    <div className="space-y-4">
      <Card className="space-y-2 px-4 py-3">
        <div className="flex flex-wrap items-center gap-2">
          <Segmented label={t("common.timeRange")} value={range} onChange={setRange} options={ranges} />
          <Segmented
            label={t("spend.metric")}
            value={metric}
            onChange={setMetric}
            options={[
              { value: "tokens", label: t("spend.metric.tokens") },
              { value: "usd", label: t("spend.metric.usd") },
            ]}
          />
          <Button className="ml-auto h-8" disabled={!report || loading} onClick={() => saveFile(() => invoke<string>("export_spending_csv", { range }))}>
            <Download className="h-4 w-4" aria-hidden />
            {t("spend.csv")}
          </Button>
        </div>
        <p className={MUTED}>{t("spend.info")}</p>
      </Card>

      <Budget automation={automation} onDashboard={onDashboard} report={report} />

      {loading && progress && progress.total > 0 && (
        <div className="space-y-1.5 px-1">
          <p className="text-xs tabular-nums text-zinc-500 dark:text-zinc-400">{t("spend.progress", { done: progress.done, n: progress.total })}</p>
          <div
            className="h-1.5 w-full overflow-hidden rounded-full bg-zinc-200/70 dark:bg-white/10"
            role="progressbar"
            aria-label={t("spend.loading")}
            aria-valuenow={progress.done}
            aria-valuemin={0}
            aria-valuemax={progress.total}
          >
            <div className={`h-full rounded-full transition-[width] duration-200 ${BAR}`} style={{ width: `${(progress.done / progress.total) * 100}%` }} />
          </div>
        </div>
      )}

      {error && (
        <p role="alert" className="text-sm text-rose-700 dark:text-rose-300">
          {error}
        </p>
      )}

      {!report ? (
        <div className="space-y-4" aria-busy="true" aria-label={t("spend.loading")}>
          <div className="grid grid-cols-3 gap-3">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-[104px] animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
            ))}
          </div>
          <div className="h-64 animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
        </div>
      ) : report.totals.messages === 0 ? (
        <Card className={`flex flex-col items-center justify-center px-4 py-10 text-center transition-opacity ${loading ? "opacity-60" : ""}`}>
          <Coins className="mb-2 h-6 w-6 text-zinc-400" aria-hidden />
          <p className="text-sm text-zinc-600 dark:text-zinc-300">{t("spend.empty")}</p>
          <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">{t("spend.emptyHint")}</p>
        </Card>
      ) : (
        <div className={`space-y-4 transition-opacity ${loading ? "opacity-60" : ""}`} aria-busy={loading}>
          <Overview report={report} metric={metric} />
          <UsageChart report={report} metric={metric} />
          <Tables report={report} metric={metric} filter={filter} setFilter={setFilter} project={project} setProject={setProject} />
          <Prices report={report} />
          <p className="text-xs text-zinc-500 dark:text-zinc-400">
            {t("spend.footnote")}
            {metric === "usd" && " " + t("spend.usdNote", { date: dayLabel(Date.parse(`${report.pricesAsOf}T00:00`), false) })}
            {metric === "usd" && report.totals.unpriced.total > 0 && " " + t("spend.unpriced", { n: full(report.totals.unpriced.total) })}
            {report.skippedFiles > 0 && t("spend.skipped", { n: report.skippedFiles })}
          </p>
        </div>
      )}
    </div>
  );
}

const BUDGET_MAX = 100_000;

/** Today's and the last 7 days' spending against the alert limits; edited in place. */
function Budget({ automation, onDashboard, report }: { automation: AutomationSettings; onDashboard: (d: FullDashboardPayload) => void; report: Report | null }) {
  const limits = [automation.budgetDailyUsd ?? 0, automation.budgetWeeklyUsd ?? 0];
  const on = limits.some((l) => l > 0);
  const [spent, setSpent] = useState<BudgetSpent | null>(null);
  const [draft, setDraft] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Fetched after each page report (the scan is shared, so this only reads what is new): the "today"
  // and "7" totals come from the backend whatever range the page shows.
  useEffect(() => {
    if (!on || !report) return;
    let live = true;
    invoke<BudgetSpent>("get_spending_budget").then((s) => live && setSpent(s)).catch(() => {});
    return () => {
      live = false;
    };
  }, [on, report]);

  const close = () => (setDraft(null), setError(null));
  const save = async () => {
    const values = draft!.map((v) => (v.trim() === "" ? 0 : Number(v.replace(",", "."))));
    if (values.some((v) => !Number.isFinite(v) || v < 0 || v > BUDGET_MAX)) return setError(t("spend.budget.invalid"));
    setSaving(true);
    try {
      onDashboard(await invoke<FullDashboardPayload>("set_spending_budget", { daily: values[0], weekly: values[1] }));
      close();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  if (draft) {
    const fields = [t("spend.budget.daily"), t("spend.budget.weekly")];
    return (
      <Card className="space-y-2 px-4 py-3">
        <h2 className="text-sm font-medium text-zinc-900 dark:text-zinc-100">{t("spend.budget")}</h2>
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
          onKeyDown={(e) => e.key === "Escape" && close()}
        >
          {fields.map((label, i) => (
            <label key={label} className={`flex flex-col gap-1 ${MUTED}`}>
              {label}
              <input
                type="number"
                inputMode="decimal"
                min={0}
                max={BUDGET_MAX}
                step="any"
                autoFocus={i === 0}
                value={draft[i]}
                onChange={(e) => setDraft(draft.map((v, k) => (k === i ? e.target.value : v)))}
                aria-invalid={!!error}
                className="h-8 w-40 rounded-md border border-zinc-300 bg-white px-2 text-sm tabular-nums text-zinc-900 outline-none focus:border-[#D97757] focus:ring-2 focus:ring-[#D97757]/30 dark:border-white/15 dark:bg-[#17181c] dark:text-zinc-100"
              />
            </label>
          ))}
          <Button type="submit" variant="primary" className="h-8" disabled={saving}>
            {t("spend.budget.save")}
          </Button>
          <Button className="h-8" disabled={saving} onClick={close}>
            {t("common.cancel")}
          </Button>
        </form>
        {error && (
          <p role="alert" className="text-xs text-rose-700 dark:text-rose-300">
            {error}
          </p>
        )}
        <p className={MUTED}>{t("spend.budget.hint")}</p>
      </Card>
    );
  }

  const edit = () => setDraft(limits.map((l) => (l > 0 ? String(l) : "")));
  if (!on) {
    return (
      <Card className="flex flex-wrap items-center gap-2 px-4 py-2.5">
        <p className={`min-w-0 flex-1 ${MUTED}`}>{t("spend.budget.offLine")}</p>
        <Button className="h-8" onClick={edit}>
          {t("spend.budget.set")}
        </Button>
      </Card>
    );
  }

  const rows = [t("spend.budget.today"), t("spend.budget.week")].map((label, i) => ({ label, limit: limits[i], value: spent ? (i === 0 ? spent.today : spent.week) : null }));
  return (
    <Card className="px-4 py-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-sm font-medium text-zinc-900 dark:text-zinc-100">{t("spend.budget")}</h2>
        <Button variant="ghost" className="h-7 px-2 text-xs" onClick={edit}>
          {t("spend.budget.edit")}
        </Button>
      </div>
      <dl className="grid grid-cols-2 gap-4">
        {rows.map(({ label, limit, value }) => {
          const share = limit > 0 && value !== null ? (value / limit) * 100 : 0;
          // Neutral below 80 %, amber from 80 %, red at 100 %; the percentage is always written out too.
          const fill = share >= 100 ? "bg-rose-500" : share >= 80 ? "bg-amber-400" : "bg-zinc-400 dark:bg-zinc-500";
          const pctText = t("fmt.pct", { n: Math.floor(share).toLocaleString(locale()) }) + (share >= 100 ? ` · ${t("spend.budget.over")}` : "");
          const shown = value === null ? "…" : usdFull(value);
          return (
            <div key={label} className="min-w-0 space-y-1">
              <dt className={`flex justify-between gap-2 ${MUTED}`}>
                <span className="truncate">{label}</span>
                {limit > 0 && value !== null && <span className={`tabular-nums ${share >= 100 ? "font-medium text-rose-700 dark:text-rose-300" : ""}`}>{pctText}</span>}
              </dt>
              <dd className="text-sm font-medium tabular-nums text-zinc-900 dark:text-zinc-100">
                {limit > 0 ? (
                  <>
                    {shown} <span className="font-normal text-zinc-500 dark:text-zinc-400">/ {usdFull(limit)}</span>
                  </>
                ) : (
                  <span className="font-normal text-zinc-500 dark:text-zinc-400">{t("spend.budget.rowOff")}</span>
                )}
              </dd>
              {limit > 0 && (
                <div
                  className="h-1 w-full overflow-hidden rounded-full bg-zinc-200/70 dark:bg-white/10"
                  role="progressbar"
                  aria-label={t("spend.budget.aria", { label, spent: shown, limit: usdFull(limit), pct: pctText })}
                  aria-valuenow={Math.round(Math.min(share, 100))}
                  aria-valuemin={0}
                  aria-valuemax={100}
                >
                  <div className={`h-full rounded-full transition-[width] duration-300 ${fill}`} style={{ width: `${Math.min(100, share)}%` }} />
                </div>
              )}
            </div>
          );
        })}
      </dl>
    </Card>
  );
}

/** Headline, breakdown and the summary numbers. */
function Overview({ report: r, metric }: { report: Report; metric: Metric }) {
  const tk = r.totals.tokens;
  const total = metric === "usd" ? r.totals.cost.total : tk.work;
  const prev = r.previous && (metric === "usd" ? r.previous.cost : r.previous.tokens.work);
  const delta = prev ? (total - prev) / prev : null;
  const top = [...r.projects].sort((a, b) => (metric === "usd" ? b.cost - a.cost : b.tokens.work - a.tokens.work))[0];
  const sub = metric === "usd" ? r.totals.subagentCost : r.totals.subagentTokens.work;
  const dayTotals = r.days.map((d) => FAMILIES.reduce((s, f) => s + amount(metric, d.families[f]), 0));
  const busiestDay = dayTotals.reduce((best, v, i) => (v > dayTotals[best] ? i : best), 0);
  const hourVals = r.hours.map((h) => amount(metric, h));
  const busiestHour = hourVals.reduce((best, v, i) => (v > hourVals[best] ? i : best), 0);
  const inputSide = tk.input + tk.cacheWrite + tk.cacheRead;
  const parts =
    metric === "usd"
      ? ([["spend.input", r.totals.cost.input], ["spend.output", r.totals.cost.output], ["spend.cacheWrite", r.totals.cost.cacheWrite], ["spend.cacheRead", r.totals.cost.cacheRead]] as const)
      : ([["spend.input", tk.input], ["spend.output", tk.output], ["spend.cacheWrite", tk.cacheWrite], ["spend.cacheRead", tk.cacheRead]] as const);
  const deltaText = delta === null ? "" : (delta >= 0 ? "+" : "−") + t("fmt.pct", { n: Math.round(Math.abs(delta) * 100).toLocaleString(locale()) });
  const DeltaIcon = delta !== null && delta < 0 ? ArrowDown : ArrowUp;
  const stats: [string, string, string?][] = [
    [t("spend.dailyAvg"), fmtMetric(metric, total / Math.max(1, r.days.length))],
    [t("spend.perChat"), fmtMetric(metric, total / Math.max(1, r.totals.chats))],
    [t("spend.cacheEff"), pct(tk.cacheRead, inputSide), t("spend.cacheEffTip")],
    [t("spend.busiestDay"), dayTotals[busiestDay] > 0 ? `${dayLabel(r.days[busiestDay].start)} · ${fmtMetric(metric, dayTotals[busiestDay])}` : "—"],
    [t("spend.busiestHour"), hourVals[busiestHour] > 0 ? `${busiestHour}:00–${busiestHour + 1}:00` : "—"],
  ];
  const maxHour = Math.max(...hourVals, 0);

  return (
    <>
      <div className="grid grid-cols-3 gap-3">
        <StatCard
          label={metric === "usd" ? t("spend.usdHeadline") : t("spend.work")}
          value={<span title={fullMetric(metric, total)}>{metric === "usd" ? `≈ ${usd(total)}` : compact(total)}</span>}
          icon={Coins}
          tone="sky"
          sub={
            metric === "usd" ? (
              <span>{t("spend.tokens", { n: compact(tk.work) })}</span>
            ) : (
              <span title={t("spend.cacheReads", { n: full(tk.cacheRead) })}>+ {t("spend.cacheReads", { n: compact(tk.cacheRead) })}</span>
            )
          }
        >
          {delta !== null && (
            <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-white/70 px-1.5 py-0.5 text-xs font-medium tabular-nums text-zinc-700 ring-1 ring-zinc-200 dark:bg-white/5 dark:text-zinc-300 dark:ring-white/10" title={t("spend.vsPrev", { pct: deltaText })}>
              <DeltaIcon className="h-3 w-3" aria-hidden />
              <span className="sr-only">{t("spend.vsPrev", { pct: deltaText })}</span>
              <span aria-hidden>{deltaText}</span>
            </span>
          )}
        </StatCard>
        <StatCard label={t("spend.topProject")} value={<span title={top?.cwd}>{top ? folderName(top.cwd) : "—"}</span>} icon={FolderOpen} tone="amber" small />
        <StatCard label={t("spend.chats")} value={full(r.totals.chats)} icon={MessagesSquare} tone="green" sub={t("spend.subagents", { pct: pct(sub, total) })} />
      </div>

      <Card className="px-4 py-3">
        <h2 className="sr-only">{t("spend.breakdown")}</h2>
        <dl className="grid grid-cols-4 gap-4">
          {parts.map(([key, n]) => (
            <div key={key} className="min-w-0">
              <dt className={`truncate ${MUTED}`}>{t(key)}</dt>
              <dd className={`mt-0.5 text-sm font-medium tabular-nums ${key === "spend.cacheRead" ? "text-zinc-500 dark:text-zinc-400" : "text-zinc-900 dark:text-zinc-100"}`} title={fullMetric(metric, n)}>
                {fmtMetric(metric, n)}
              </dd>
            </div>
          ))}
        </dl>
      </Card>

      <Card className="space-y-3 px-4 py-3">
        <h2 className="sr-only">{t("spend.summary")}</h2>
        <dl className="grid grid-cols-5 gap-4">
          {stats.map(([label, value, tip]) => (
            <div key={label} className="min-w-0" title={tip}>
              <dt className={`truncate ${MUTED}`}>{label}</dt>
              <dd className="mt-0.5 truncate text-sm font-medium tabular-nums text-zinc-900 dark:text-zinc-100">{value}</dd>
            </div>
          ))}
        </dl>
        <div>
          <p className={`mb-1 ${MUTED}`}>{t("spend.hours")}</p>
          <div className="flex h-8 items-end gap-[2px]" role="img" aria-label={`${t("spend.hours")}: ${hourVals.map((v, h) => t("spend.hourAria", { h, v: fmtMetric(metric, v) })).join(", ")}`}>
            {hourVals.map((v, h) => (
              <div key={h} className="flex h-full flex-1 items-end rounded-t-[2px] bg-zinc-100 dark:bg-white/[0.04]" title={t("spend.hourAria", { h, v: fullMetric(metric, v) })}>
                <div className={`w-full rounded-t-[2px] ${BAR}`} style={{ height: `${maxHour > 0 && v > 0 ? Math.max(6, (v / maxHour) * 100) : 0}%` }} />
              </div>
            ))}
          </div>
          <div className="mt-1 grid grid-cols-4 text-[10px] tabular-nums text-zinc-500 dark:text-zinc-400" aria-hidden>
            {[0, 6, 12, 18].map((h) => (
              <span key={h}>{h}:00</span>
            ))}
          </div>
        </div>
      </Card>
    </>
  );
}

/** Daily bars (weekly when the range is long), stacked by model family, with a hover/focus tooltip. */
function UsageChart({ report, metric }: { report: Report; metric: Metric }) {
  const weekly = report.days.length > 92;
  const buckets = useMemo(() => {
    const out: { start: number; fam: Record<string, Amount> }[] = [];
    for (const d of report.days) {
      const monday = new Date(d.start);
      monday.setDate(monday.getDate() - ((monday.getDay() + 6) % 7));
      const start = weekly ? monday.getTime() : d.start;
      if (out[out.length - 1]?.start !== start) out.push({ start, fam: {} });
      const b = out[out.length - 1];
      for (const [f, a] of Object.entries(d.families)) {
        const s = (b.fam[f] ??= { work: 0, cost: 0 });
        s.work += a.work;
        s.cost += a.cost;
      }
    }
    return out;
  }, [report, weekly]);
  const [hover, setHover] = useState<number | null>(null);
  const totals = buckets.map((b) => FAMILIES.reduce((s, f) => s + amount(metric, b.fam[f]), 0));
  const max = Math.max(...totals, 0);
  const present = FAMILIES.filter((f) => buckets.some((b) => amount(metric, b.fam[f]) > 0));
  const title = weekly ? t("spend.chartWeekly") : t("spend.chartDaily");
  const label = (i: number) => (weekly ? t("spend.weekOf", { date: dayLabel(buckets[i].start, false) }) : dayLabel(buckets[i].start));

  const W = 720, H = 190, top = 10, bottom = 22, left = 52;
  const plotH = H - top - bottom;
  const slot = (W - left) / Math.max(1, buckets.length);
  const bw = Math.max(1, Math.min(32, slot - 2));
  const y = (v: number) => (max > 0 ? (v / max) * plotH : 0);
  const base = top + plotH;
  const ticks = [0, 1, 2].map((i) => (max * i) / 2);
  const xLabels = [...new Set([0, Math.floor((buckets.length - 1) / 2), buckets.length - 1])];

  return (
    <Card className="px-4 py-4">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-medium text-zinc-900 dark:text-zinc-100">{title}</h2>
        <ul className="flex flex-wrap gap-3 text-xs text-zinc-600 dark:text-zinc-300">
          {present.map((f) => (
            <li key={f} className="flex items-center gap-1.5">
              <span className={`h-2.5 w-2.5 rounded-sm ${FAMILY_BG[f]}`} aria-hidden />
              {familyName(f)}
            </li>
          ))}
        </ul>
      </div>
      <div className="relative">
        <svg viewBox={`0 0 ${W} ${H}`} className="h-auto w-full" role="group" aria-label={t("spend.chartAria", { title, n: buckets.length })}>
          {ticks.map((v, i) => (
            <g key={i}>
              <line x1={left} x2={W} y1={base - y(v)} y2={base - y(v)} className="stroke-zinc-200 dark:stroke-white/10" strokeWidth={1} />
              <text x={left - 6} y={base - y(v) + 3} textAnchor="end" className="fill-zinc-500 text-[10px] tabular-nums dark:fill-zinc-400">
                {fmtMetric(metric, v)}
              </text>
            </g>
          ))}
          {buckets.map((b, i) => {
            const x = left + i * slot + (slot - bw) / 2;
            const segs = FAMILIES.map((f) => ({ f, h: y(amount(metric, b.fam[f])) })).filter((s) => s.h > 0);
            let cursor = base;
            return (
              <g key={b.start} className={hover !== null && hover !== i ? "opacity-60" : undefined}>
                {segs.map((s, k) => {
                  const isTop = k === segs.length - 1;
                  const yTop = cursor - s.h;
                  // 2px surface gap above every segment but the top one; rounded data end on top.
                  const h = Math.max(0, s.h - (isTop ? 0 : 2));
                  cursor = yTop;
                  const r = isTop ? Math.min(4, bw / 2, h) : 0;
                  const d = `M${x},${yTop + h} V${yTop + r} Q${x},${yTop} ${x + r},${yTop} H${x + bw - r} Q${x + bw},${yTop} ${x + bw},${yTop + r} V${yTop + h} Z`;
                  return <path key={s.f} d={d} className={FAMILY_FILL[s.f]} />;
                })}
                <rect
                  x={left + i * slot}
                  y={top}
                  width={slot}
                  height={plotH}
                  fill="transparent"
                  tabIndex={0}
                  className="outline-none focus-visible:stroke-zinc-400"
                  aria-label={`${label(i)}: ${fullMetric(metric, totals[i])}`}
                  onPointerEnter={() => setHover(i)}
                  onPointerLeave={() => setHover(null)}
                  onFocus={() => setHover(i)}
                  onBlur={() => setHover(null)}
                />
              </g>
            );
          })}
          <line x1={left} x2={W} y1={base} y2={base} className="stroke-zinc-300 dark:stroke-white/20" strokeWidth={1} />
          {xLabels.map((i) => (
            <text key={i} x={left + i * slot + slot / 2} y={H - 6} textAnchor={i === 0 && buckets.length > 1 ? "start" : i === buckets.length - 1 && buckets.length > 1 ? "end" : "middle"} className="fill-zinc-500 text-[10px] dark:fill-zinc-400">
              {weekly ? dayLabel(buckets[i].start, false) : dayLabel(buckets[i].start)}
            </text>
          ))}
        </svg>
        {hover !== null && (
          <div
            className="pointer-events-none absolute top-0 z-10 w-48 -translate-x-1/2 rounded-lg border border-zinc-200 bg-white px-3 py-2 text-xs shadow-lg dark:border-white/10 dark:bg-[#202127]"
            style={{ left: `clamp(6rem, ${((left + hover * slot + slot / 2) / W) * 100}%, calc(100% - 6rem))` }}
            role="tooltip"
          >
            <p className="mb-1 text-zinc-500 dark:text-zinc-400">{label(hover)}</p>
            <p className="mb-1.5 font-semibold tabular-nums text-zinc-900 dark:text-zinc-100">{fullMetric(metric, totals[hover])}</p>
            <ul className="space-y-0.5">
              {FAMILIES.filter((f) => amount(metric, buckets[hover].fam[f]) > 0).map((f) => (
                <li key={f} className="flex items-center gap-2">
                  <span className={`h-0.5 w-3 shrink-0 rounded-full ${FAMILY_BG[f]}`} aria-hidden />
                  <span className="font-medium tabular-nums text-zinc-900 dark:text-zinc-100">{fmtMetric(metric, amount(metric, buckets[hover].fam[f]))}</span>
                  <span className="text-zinc-500 dark:text-zinc-400">{familyName(f)}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </Card>
  );
}

type Col<R> = { key: string; label: string; sort?: (r: R) => number | string; cell: (r: R) => React.ReactNode; className: string };

/** A table with sortable columns and the top rows until "show all". `metric` resets the sort to its column. */
function SortTable<R>({ title, rows, cols, rowKey, metric, extra }: { title: string; rows: R[]; cols: Col<R>[]; rowKey: (r: R) => string; metric: Metric; extra?: React.ReactNode }) {
  const [sort, setSort] = useState<{ key: string; desc: boolean }>({ key: metric, desc: true });
  const [all, setAll] = useState(false);
  useEffect(() => setSort({ key: metric, desc: true }), [metric]);
  const sorted = useMemo(() => {
    const by = cols.find((c) => c.key === sort.key)?.sort;
    if (!by) return rows;
    const dir = sort.desc ? -1 : 1;
    return [...rows].sort((a, b) => {
      const [x, y] = [by(a), by(b)];
      return (typeof x === "string" ? x.localeCompare(String(y), locale()) : x - (y as number)) * dir;
    });
  }, [rows, cols, sort]);
  const shown = all ? sorted : sorted.slice(0, TOP);
  return (
    <Card className="overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 px-4 pb-2 pt-4">
        <h2 className="mr-auto text-sm font-medium text-zinc-900 dark:text-zinc-100">{title}</h2>
        {extra}
      </div>
      <table className="w-full table-fixed text-sm">
        <thead>
          <tr className="border-b border-zinc-100 text-left text-xs text-zinc-500 dark:border-white/[0.06] dark:text-zinc-400">
            {cols.map((c) => {
              const active = sort.key === c.key;
              return (
                <th key={c.key} scope="col" className={`py-2 font-medium ${c.className}`} aria-sort={active ? (sort.desc ? "descending" : "ascending") : undefined}>
                  {c.sort ? (
                    <button
                      type="button"
                      title={t("spend.sortBy", { col: c.label })}
                      onClick={() => setSort({ key: c.key, desc: active ? !sort.desc : true })}
                      className={`inline-flex items-center gap-1 rounded hover:text-zinc-800 dark:hover:text-zinc-200 ${active ? "text-zinc-900 dark:text-zinc-100" : ""} ${focusRing}`}
                    >
                      {c.label}
                      {active && (sort.desc ? <ArrowDown className="h-3 w-3" aria-hidden /> : <ArrowUp className="h-3 w-3" aria-hidden />)}
                    </button>
                  ) : (
                    c.label
                  )}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {shown.map((r) => (
            <tr key={rowKey(r)} className="border-b border-zinc-100 last:border-0 dark:border-white/[0.06]">
              {cols.map((c) => (
                <td key={c.key} className={`truncate py-2 ${c.className}`}>
                  {c.cell(r)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <p className={`px-4 py-3 ${MUTED}`}>{t("spend.noMatch")}</p>}
      {rows.length > TOP && (
        <button type="button" onClick={() => setAll(!all)} className={`w-full border-t border-zinc-100 px-4 py-2 text-left text-xs font-medium text-zinc-600 hover:bg-zinc-50 dark:border-white/[0.06] dark:text-zinc-300 dark:hover:bg-white/[0.03] ${focusRing}`}>
          {all ? t("spend.showTop", { n: TOP }) : t("spend.showAll", { n: rows.length })}
        </button>
      )}
    </Card>
  );
}

function Tables({ report: r, metric, filter, setFilter, project, setProject }: { report: Report; metric: Metric; filter: string; setFilter: (s: string) => void; project: string | null; setProject: (p: string | null) => void }) {
  const q = filter.trim().toLocaleLowerCase(locale());
  const has = (...s: string[]) => !q || s.some((x) => x.toLocaleLowerCase(locale()).includes(q));
  const total = metric === "usd" ? r.totals.cost.total : r.totals.tokens.work;
  const value = (x: { tokens: { work: number }; cost: number | null }) => (metric === "usd" ? x.cost ?? 0 : x.tokens.work);
  const num = "px-2 text-right tabular-nums";
  const tokenCell = (x: { tokens: TokenSums }) => (
    <span className="text-zinc-900 dark:text-zinc-100" title={`${t("spend.tokens", { n: full(x.tokens.work) })} · ${t("spend.cacheReads", { n: full(x.tokens.cacheRead) })}`}>
      {compact(x.tokens.work)}
    </span>
  );
  const costCell = (c: number | null) =>
    c === null ? <span className={MUTED}>{t("spend.noPrice")}</span> : <span className="text-zinc-900 dark:text-zinc-100" title={usdFull(c)}>{usd(c)}</span>;
  const tail = <R extends { tokens: TokenSums; cost: number | null; messages: number; lastActivityAt: number }>(): Col<R>[] => [
    { key: "tokens", label: t("spend.metric.tokens"), sort: (x) => x.tokens.work, cell: tokenCell, className: `w-24 ${num}` },
    { key: "usd", label: t("spend.colCost"), sort: (x) => x.cost ?? -1, cell: (x) => costCell(x.cost), className: `w-24 ${num}` },
    { key: "messages", label: t("spend.colMessages"), sort: (x) => x.messages, cell: (x) => <span className="text-zinc-700 dark:text-zinc-300">{full(x.messages)}</span>, className: `w-20 ${num}` },
    { key: "last", label: t("spend.colLast"), sort: (x) => x.lastActivityAt, cell: (x) => <span className={MUTED}>{formatDateShort(x.lastActivityAt)}</span>, className: "w-32 pl-2 pr-4 text-right tabular-nums" },
  ];
  type P = Report["projects"][number];
  type C = Report["chats"][number];
  type M = Report["models"][number];
  const projects = r.projects.filter((p) => has(p.cwd));
  const chats = r.chats.filter((c) => (!project || c.cwd === project) && has(c.title, c.cwd));
  const models = r.models.filter((m) => has(m.model));

  return (
    <>
      <label className="flex items-center gap-2 rounded-lg border border-zinc-200 bg-white px-3 dark:border-white/10 dark:bg-[#17181c]">
        <Search className="h-4 w-4 shrink-0 text-zinc-400" aria-hidden />
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("spend.filter")}
          aria-label={t("spend.filter")}
          className="h-9 w-full bg-transparent text-sm text-zinc-900 outline-none placeholder:text-zinc-400 dark:text-zinc-100"
        />
      </label>

      <SortTable<P>
        title={t("spend.projects")}
        rows={projects}
        rowKey={(p) => p.cwd}
        metric={metric}
        cols={[
          {
            key: "name",
            label: t("spend.colProject"),
            sort: (p) => folderName(p.cwd),
            className: "pl-4 pr-2",
            cell: (p) => (
              <button type="button" onClick={() => setProject(p.cwd)} title={`${p.cwd}\n${t("count.chats", { n: p.chats })} · ${t("spend.pickProject")}`} className={`block w-full min-w-0 rounded text-left ${focusRing}`}>
                <span className="block truncate text-zinc-800 hover:underline dark:text-zinc-200">
                  {folderName(p.cwd)} <span className={MUTED}>{pct(value(p), total)}</span>
                </span>
                <span className="mt-1 block h-1 rounded-full bg-zinc-100 dark:bg-white/[0.06]" aria-hidden>
                  <span className={`block h-full rounded-full ${BAR}`} style={{ width: `${total > 0 ? Math.max(0.5, (value(p) / total) * 100) : 0}%` }} />
                </span>
              </button>
            ),
          },
          ...tail<P>(),
        ]}
      />

      <SortTable<C>
        title={t("spend.chatsTitle")}
        rows={chats}
        rowKey={(c) => c.cliSessionId}
        metric={metric}
        extra={
          project && (
            <span className="inline-flex max-w-[16rem] items-center gap-1 rounded-full bg-zinc-100 py-0.5 pl-2.5 pr-1 text-xs text-zinc-700 dark:bg-white/[0.06] dark:text-zinc-300" title={project}>
              <span className="truncate">{t("spend.projectChip", { name: folderName(project) })}</span>
              <button type="button" onClick={() => setProject(null)} aria-label={t("spend.clearChip")} title={t("spend.clearChip")} className={`rounded-full p-0.5 hover:bg-zinc-200 dark:hover:bg-white/10 ${focusRing}`}>
                <X className="h-3 w-3" aria-hidden />
              </button>
            </span>
          )
        }
        cols={[
          { key: "name", label: t("spend.colChat"), sort: (c) => c.title, className: "pl-4 pr-2", cell: (c) => <span className="text-zinc-800 dark:text-zinc-200" title={c.title}>{c.title}</span> },
          { key: "folder", label: t("spend.colFolder"), sort: (c) => folderName(c.cwd), className: "w-28 px-2", cell: (c) => <span className="text-zinc-500 dark:text-zinc-400" title={c.cwd}>{c.cwd ? folderName(c.cwd) : "—"}</span> },
          ...tail<C>(),
        ]}
      />

      <SortTable<M>
        title={t("spend.models")}
        rows={models}
        rowKey={(m) => m.model}
        metric={metric}
        cols={[
          {
            key: "name",
            label: t("spend.colModel"),
            sort: (m) => m.model,
            className: "pl-4 pr-2",
            cell: (m) => (
              <span className="flex min-w-0 items-center gap-2 text-zinc-800 dark:text-zinc-200" title={m.model}>
                <span className={`h-2.5 w-2.5 shrink-0 rounded-sm ${FAMILY_BG[m.family] ?? FAMILY_BG.other}`} aria-hidden />
                <span className="truncate">{m.model}</span>
              </span>
            ),
          },
          ...tail<M>(),
        ]}
      />
    </>
  );
}

function Prices({ report: r }: { report: Report }) {
  const price = (n: number) => new Intl.NumberFormat(locale(), { style: "currency", currency: "USD", maximumFractionDigits: 3 }).format(n);
  return (
    <Card>
      <details className="group">
        <summary className={`flex cursor-pointer list-none items-center gap-2 rounded-xl px-4 py-3 text-sm font-medium text-zinc-900 dark:text-zinc-100 ${focusRing}`}>
          <ChevronDown className="h-4 w-4 text-zinc-400 transition group-open:rotate-180" aria-hidden />
          {t("spend.prices")}
        </summary>
        <div className="px-4 pb-4">
          <p className={`mb-2 ${MUTED}`}>{t("spend.pricesNote", { date: dayLabel(Date.parse(`${r.pricesAsOf}T00:00`), false) })}</p>
          <table className="w-full table-fixed text-sm">
            <thead>
              <tr className="border-b border-zinc-100 text-left text-xs text-zinc-500 dark:border-white/[0.06] dark:text-zinc-400">
                <th scope="col" className="py-1.5 pr-2 font-medium">{t("spend.colModel")}</th>
                {(["spend.input", "spend.output", "spend.cacheWrite", "spend.cacheRead"] as const).map((k) => (
                  <th key={k} scope="col" className="w-28 px-2 py-1.5 text-right font-medium">{t(k)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {r.prices.map((p) => (
                <tr key={p.model} className="border-b border-zinc-100 last:border-0 dark:border-white/[0.06]">
                  <td className="truncate py-1.5 pr-2 text-zinc-800 dark:text-zinc-200">{p.model}</td>
                  {[p.input, p.output, p.cacheWrite, p.cacheRead].map((v, i) => (
                    <td key={i} className="px-2 py-1.5 text-right tabular-nums text-zinc-700 dark:text-zinc-300">{price(v)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </Card>
  );
}

