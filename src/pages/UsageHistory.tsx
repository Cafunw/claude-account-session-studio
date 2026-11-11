import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TrendingUp } from "lucide-react";
import { AccountProfile, UsageHistory, UsageSample } from "../types";
import { accountName, Avatar, Badge, Card, Segmented } from "../ui";
import { isUnknownPlan, locale, pct, t } from "../i18n";

const DAY = 86_400_000;
// Categorical slots 1 and 3 of the dataviz reference palette, checked with its validator in both themes.
const WEEKLY = "stroke-[#2a78d6] dark:stroke-[#3987e5]";
const FIVE_H = "stroke-[#1baf7a] dark:stroke-[#199e70]";
const W = 720;
const PLOT = { l: 36, r: 12, t: 12, b: 26 };
const PH = 190;

const fmtTime = (t: number) =>
  new Date(t).toLocaleString(locale(), { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

export default function UsageHistoryPage({ accounts, threshold }: { accounts: AccountProfile[]; threshold: number }) {
  const [days, setDays] = useState(7);
  const [hist, setHist] = useState<Record<string, UsageHistory>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const uuids = accounts.map((a) => a.accountUuid).join(",");

  useEffect(() => {
    let live = true;
    setLoading(true);
    Promise.all(accounts.map((a) => invoke<UsageHistory>("get_usage_history", { accountUuid: a.accountUuid, days })))
      .then((rs) => live && (setHist(Object.fromEntries(rs.map((r, i) => [accounts[i].accountUuid, r]))), setError(null)))
      .catch((e) => live && setError(String(e)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [days, uuids]);

  return (
    <div className="space-y-4">
      <Card className="flex items-center justify-between gap-4 px-4 py-3">
        <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("hist.info")}</p>
        <Segmented
          label={t("common.timeRange")}
          value={days}
          onChange={setDays}
          options={[7, 14, 30].map((n) => ({ value: n, label: t("fmt.days", { n }) }))}
        />
      </Card>

      {error && (
        <p role="alert" className="text-sm text-rose-700 dark:text-rose-300">
          {error}
        </p>
      )}
      {accounts.length === 0 && (
        <Card className="py-8 text-center text-sm text-zinc-500 dark:text-zinc-400">{t("common.noAccounts")}</Card>
      )}

      {/* Refetch keeps the previous charts, dimmed, instead of a skeleton. */}
      <div className={`space-y-4 transition-opacity ${loading ? "opacity-60" : ""}`} aria-busy={loading}>
        {accounts.map((acc) => {
          const { name, hint } = accountName(acc);
          const h = hist[acc.accountUuid];
          return (
            <Card key={acc.accountUuid} className="px-4 py-4">
              <div className="mb-3 flex items-center gap-3">
                <Avatar name={name} seed={acc.accountUuid} />
                <div className="min-w-0 flex-1">
                  <p className="flex items-center gap-2 truncate text-sm font-medium text-zinc-900 dark:text-zinc-100">
                    <span className="truncate">{name}</span>
                    {!isUnknownPlan(acc.planType) && <Badge>{acc.planType}</Badge>}
                  </p>
                  {hint && <p className="truncate text-xs text-zinc-500 dark:text-zinc-400">{hint}</p>}
                </div>
                {h && h.samples.length > 0 && <Legend samples={h.samples} />}
              </div>
              {!h ? (
                <div className="h-[216px] animate-pulse rounded-lg bg-zinc-100 dark:bg-white/[0.04]" />
              ) : h.samples.length === 0 ? (
                <div className="flex h-[140px] flex-col items-center justify-center rounded-lg border border-dashed border-zinc-200 text-center dark:border-white/10">
                  <p className="text-sm text-zinc-600 dark:text-zinc-300">{t("hist.noData")}</p>
                  <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">{t("hist.noDataHint")}</p>
                </div>
              ) : (
                <>
                  <Chart h={h} days={days} threshold={threshold} label={name} />
                  <p className="mt-3 flex items-center gap-2 text-sm text-zinc-700 dark:text-zinc-300">
                    <TrendingUp className="h-4 w-4 shrink-0 text-zinc-400" aria-hidden />
                    {h.forecast}
                  </p>
                </>
              )}
            </Card>
          );
        })}
      </div>
    </div>
  );
}

function lastOf(samples: UsageSample[], k: "sd" | "fh") {
  for (let i = samples.length - 1; i >= 0; i--) if (samples[i][k] != null) return samples[i][k]!;
  return null;
}

function Legend({ samples }: { samples: UsageSample[] }) {
  const sd = lastOf(samples, "sd");
  const fh = lastOf(samples, "fh");
  const item = (cls: string, label: string, v: number | null) => (
    <span className="flex items-center gap-1.5">
      <svg width="14" height="4" aria-hidden>
        <line x1="1" y1="2" x2="13" y2="2" strokeWidth="2" strokeLinecap="round" className={cls} />
      </svg>
      {label} <span className="font-medium tabular-nums text-zinc-900 dark:text-zinc-100">{pct(v!)}</span>
    </span>
  );
  return (
    <div className="flex shrink-0 items-center gap-4 text-xs text-zinc-500 dark:text-zinc-400">
      {sd != null && item(WEEKLY, t("quota.weekly"), sd)}
      {fh != null && item(FIVE_H, t("quota.fiveHour"), fh)}
    </div>
  );
}

function Chart({ h, days, threshold, label }: { h: UsageHistory; days: number; threshold: number; label: string }) {
  const svg = useRef<SVGSVGElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const now = Date.now();
  const x0 = now - days * DAY;
  const x = (t: number) => PLOT.l + ((t - x0) / (now - x0)) * (W - PLOT.l - PLOT.r);
  const y = (p: number) => PLOT.t + (1 - Math.min(100, Math.max(0, p)) / 100) * PH;
  const H = PLOT.t + PH + PLOT.b;
  const s = h.samples;

  const path = (k: "sd" | "fh") =>
    s.reduce((d, p, i) => (p[k] == null ? d : `${d}${d && s[i - 1]?.[k] != null ? "L" : "M"}${x(p.t).toFixed(1)},${y(p[k]!).toFixed(1)}`), "");
  const hasFh = s.some((p) => p.fh != null);

  const step = days <= 7 ? 1 : days <= 14 ? 2 : 5;
  const ticks: number[] = [];
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  for (let t = d.getTime(), i = 0; t > x0; d.setDate(d.getDate() - 1), t = d.getTime(), i++) if (i % step === 0) ticks.push(t);

  const nearest = (clientX: number) => {
    const r = svg.current!.getBoundingClientRect();
    const vx = ((clientX - r.left) / r.width) * W;
    let best = 0;
    s.forEach((p, i) => Math.abs(x(p.t) - vx) < Math.abs(x(s[best].t) - vx) && (best = i));
    return best;
  };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    setHover((i) => Math.min(s.length - 1, Math.max(0, (i ?? s.length) + (e.key === "ArrowLeft" ? -1 : 1))));
  };
  const hp = hover != null ? s[hover] : null;
  const sd = lastOf(s, "sd");

  return (
    <div className="relative">
      <svg
        ref={svg}
        viewBox={`0 0 ${W} ${H}`}
        className="block h-auto w-full touch-none select-none rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#D97757]/70"
        role="img"
        tabIndex={0}
        aria-label={t("hist.chartLabel", { name: label, days, now: sd != null ? t("hist.chartNow", { n: Math.round(sd) }) : "" })}
        onPointerMove={(e) => setHover(nearest(e.clientX))}
        onPointerLeave={() => setHover(null)}
        onKeyDown={onKey}
        onBlur={() => setHover(null)}
      >
        {[0, 25, 50, 75, 100].map((p) => (
          <g key={p}>
            <line x1={PLOT.l} x2={W - PLOT.r} y1={y(p)} y2={y(p)} className="stroke-zinc-100 dark:stroke-white/[0.06]" strokeWidth={1} />
            <text x={PLOT.l - 6} y={y(p)} dy="0.32em" textAnchor="end" className="fill-zinc-400 text-[10px] tabular-nums dark:fill-zinc-500">
              {p}
            </text>
          </g>
        ))}
        {ticks.map((t) => (
          <text key={t} x={x(t)} y={H - 8} textAnchor="middle" className="fill-zinc-400 text-[10px] dark:fill-zinc-500">
            {new Date(t).toLocaleDateString(locale(), { day: "numeric", month: "short" })}
          </text>
        ))}
        {h.resets.map((at) => (
          <g key={at}>
            <line x1={x(at)} x2={x(at)} y1={PLOT.t} y2={PLOT.t + PH} className="stroke-zinc-300 dark:stroke-white/20" strokeWidth={1} />
            {/* Near the right edge the label goes left of the line, clear of the threshold label. */}
            <text x={x(at) > W - 140 ? x(at) - 4 : x(at) + 4} textAnchor={x(at) > W - 140 ? "end" : "start"} y={PLOT.t + 9} className="fill-zinc-400 text-[10px] dark:fill-zinc-500">
              {t("hist.reset")}
            </text>
          </g>
        ))}
        <line x1={PLOT.l} x2={W - PLOT.r} y1={y(threshold)} y2={y(threshold)} className="stroke-rose-400/80 dark:stroke-rose-400/60" strokeWidth={1} />
        <text x={W - PLOT.r} y={y(threshold) - 4} textAnchor="end" className="fill-zinc-500 text-[10px] dark:fill-zinc-400">
          {t("hist.alert", { n: threshold })}
        </text>
        {hasFh && <path d={path("fh")} fill="none" strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" className={FIVE_H} />}
        <path d={path("sd")} fill="none" strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" className={WEEKLY} />
        {hp && (
          <g pointerEvents="none">
            <line x1={x(hp.t)} x2={x(hp.t)} y1={PLOT.t} y2={PLOT.t + PH} className="stroke-zinc-400 dark:stroke-zinc-500" strokeWidth={1} />
            {(["fh", "sd"] as const).map(
              (k) =>
                hp[k] != null && (
                  <circle key={k} cx={x(hp.t)} cy={y(hp[k]!)} r={4} strokeWidth={2} className={`${k === "sd" ? "fill-[#2a78d6] dark:fill-[#3987e5]" : "fill-[#1baf7a] dark:fill-[#199e70]"} stroke-white dark:stroke-[#17181c]`} />
                )
            )}
          </g>
        )}
      </svg>
      {hp && (
        <div
          className="pointer-events-none absolute top-0 z-10 min-w-[9rem] -translate-x-1/2 rounded-lg border border-zinc-200 bg-white px-3 py-2 text-xs shadow-lg dark:border-white/10 dark:bg-[#1e1f24] dark:shadow-black/40"
          style={{ left: `${Math.min(88, Math.max(12, (x(hp.t) / W) * 100))}%` }}
          role="status"
        >
          <p className="mb-1 text-zinc-500 dark:text-zinc-400">{fmtTime(hp.t)}</p>
          {hp.sd != null && <TipRow cls={WEEKLY} label={t("quota.weekly")} v={hp.sd} />}
          {hp.fh != null && <TipRow cls={FIVE_H} label={t("quota.fiveHour")} v={hp.fh} />}
        </div>
      )}
    </div>
  );
}

function TipRow({ cls, label, v }: { cls: string; label: string; v: number }) {
  return (
    <p className="flex items-center gap-2">
      <svg width="10" height="4" aria-hidden>
        <line x1="1" y1="2" x2="9" y2="2" strokeWidth="2" strokeLinecap="round" className={cls} />
      </svg>
      <span className="font-semibold tabular-nums text-zinc-900 dark:text-zinc-100">{pct(v)}</span>
      <span className="text-zinc-500 dark:text-zinc-400">{label}</span>
    </p>
  );
}
