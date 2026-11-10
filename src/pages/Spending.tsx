import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Coins, FolderOpen, MessagesSquare } from "lucide-react";
import { SpendingReport } from "../types";
import { Card, formatDateShort, Segmented, StatCard } from "../ui";

const compact = new Intl.NumberFormat("tr-TR", { notation: "compact", maximumFractionDigits: 1 });
const full = new Intl.NumberFormat("tr-TR");
const fmt = (n: number) => compact.format(n);
const pct = (n: number, of: number) => (of > 0 ? `%${((n / of) * 100).toLocaleString("tr-TR", { maximumFractionDigits: n / of < 0.1 ? 1 : 0 })}` : "%0");
const folderName = (cwd: string) => cwd.split(/[\\/]/).filter(Boolean).pop() || "Bilinmeyen klasör";

// One magnitude series: categorical slot 1 of the dataviz reference palette (light / dark).
const BAR = "bg-[#2a78d6] dark:bg-[#3987e5]";
// Model shares: slots 1-4 in fixed order, the rest folded into a gray "Diğer".
const MODEL_COLORS = ["bg-[#2a78d6] dark:bg-[#3987e5]", "bg-[#eb6834] dark:bg-[#d95926]", "bg-[#1baf7a] dark:bg-[#199e70]", "bg-[#eda100] dark:bg-[#c98500]"];
const OTHER = "bg-zinc-400 dark:bg-zinc-500";

export default function SpendingPage() {
  const [days, setDays] = useState(7);
  const [report, setReport] = useState<SpendingReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setLoading(true);
    invoke<SpendingReport>("get_spending", { days })
      .then((r) => live && (setReport(r), setError(null)))
      .catch((e) => live && setError(String(e)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [days]);

  const total = report?.totals.tokens.total ?? 0;
  const top = report?.projects[0];

  return (
    <div className="space-y-4">
      <Card className="flex items-center justify-between gap-4 px-4 py-3">
        <p className="text-xs text-zinc-500 dark:text-zinc-400">Son {days} günde en çok token kullanan projeler, sohbetler ve modeller.</p>
        <Segmented
          label="Zaman aralığı"
          value={days}
          onChange={setDays}
          options={[
            { value: 7, label: "7 gün" },
            { value: 30, label: "30 gün" },
          ]}
        />
      </Card>

      {error && (
        <p role="alert" className="text-sm text-rose-700 dark:text-rose-300">
          {error}
        </p>
      )}

      {!report ? (
        <div className="space-y-4" aria-busy="true" aria-label="Harcama yükleniyor">
          <div className="grid grid-cols-3 gap-3">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-[84px] animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
            ))}
          </div>
          <div className="h-64 animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
        </div>
      ) : report.totals.messages === 0 ? (
        <Card className="flex flex-col items-center justify-center px-4 py-10 text-center">
          <Coins className="mb-2 h-6 w-6 text-zinc-400" aria-hidden />
          <p className="text-sm text-zinc-600 dark:text-zinc-300">Son {days} günde token kaydı yok.</p>
          <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">Claude Code sohbetleri yapıldıkça burada görünür.</p>
        </Card>
      ) : (
        <div className={`space-y-4 transition-opacity ${loading ? "opacity-60" : ""}`} aria-busy={loading}>
          <div className="grid grid-cols-3 gap-3">
            <StatCard label="Toplam token" value={<span title={full.format(total)}>{fmt(total)}</span>} icon={Coins} tone="sky" />
            <StatCard label="En yoğun proje" value={<span title={top?.cwd}>{top ? folderName(top.cwd) : "—"}</span>} icon={FolderOpen} tone="amber" small />
            <StatCard label="Sohbet" value={full.format(report.totals.chats)} icon={MessagesSquare} tone="green" />
          </div>

          <Card className="px-4 py-4">
            <h2 className="mb-3 text-sm font-medium text-zinc-900 dark:text-zinc-100">Projeler</h2>
            <ul className="space-y-2.5">
              {report.projects.map((p) => {
                const share = total > 0 ? p.tokens.total / total : 0;
                const tip = `${p.cwd}\n${full.format(p.tokens.total)} token · ${p.chats} sohbet · son: ${formatDateShort(p.lastActivityAt)}`;
                return (
                  <li key={p.cwd} title={tip} className="grid grid-cols-[minmax(0,11rem)_1fr_auto] items-center gap-3 text-sm">
                    <span className="truncate text-zinc-700 dark:text-zinc-300">{folderName(p.cwd)}</span>
                    <div
                      className="h-3"
                      role="img"
                      aria-label={`${folderName(p.cwd)} (${p.cwd}): ${full.format(p.tokens.total)} token, toplamın ${pct(p.tokens.total, total)}'i, ${p.chats} sohbet`}
                    >
                      <div className={`h-full rounded-r ${BAR}`} style={{ width: `${Math.max(0.5, share * 100)}%` }} />
                    </div>
                    <span className="w-28 text-right tabular-nums text-zinc-900 dark:text-zinc-100">
                      {fmt(p.tokens.total)} <span className="text-xs text-zinc-500 dark:text-zinc-400">{pct(p.tokens.total, total)}</span>
                    </span>
                  </li>
                );
              })}
            </ul>
          </Card>

          <div className="grid grid-cols-[minmax(0,3fr)_minmax(0,2fr)] gap-4">
            <Card className="overflow-hidden">
              <h2 className="px-4 pb-2 pt-4 text-sm font-medium text-zinc-900 dark:text-zinc-100">En çok token kullanan sohbetler</h2>
              <table className="w-full table-fixed text-sm">
                <thead>
                  <tr className="border-b border-zinc-100 text-left text-xs text-zinc-500 dark:border-white/[0.06] dark:text-zinc-400">
                    <th scope="col" className="px-4 py-2 font-medium">Sohbet</th>
                    <th scope="col" className="w-36 px-2 py-2 font-medium">Klasör</th>
                    <th scope="col" className="w-24 px-4 py-2 text-right font-medium">Token</th>
                  </tr>
                </thead>
                <tbody>
                  {report.chats.map((c) => (
                    <tr key={c.cliSessionId} className="border-b border-zinc-100 last:border-0 dark:border-white/[0.06]">
                      <td className="truncate px-4 py-2 text-zinc-800 dark:text-zinc-200" title={`${c.title} · son: ${formatDateShort(c.lastActivityAt)}`}>
                        {c.title}
                      </td>
                      <td className="truncate px-2 py-2 text-zinc-500 dark:text-zinc-400" title={c.cwd}>
                        {c.cwd ? folderName(c.cwd) : "—"}
                      </td>
                      <td className="px-4 py-2 text-right tabular-nums text-zinc-900 dark:text-zinc-100" title={`${full.format(c.tokens.total)} token`}>
                        {fmt(c.tokens.total)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </Card>

            <Models models={report.models} total={total} />
          </div>

          <p className="text-xs text-zinc-500 dark:text-zinc-400">
            Token sayıları Claude Code kayıtlarından hesaplanır; plan kotasının yüzdesi değildir ve hesaplar arasında paylaşılan sohbetler ayrılamaz.
            {report.skippedFiles > 0 && ` ${report.skippedFiles} çok büyük ya da okunamayan kayıt dosyası atlandı.`}
          </p>
        </div>
      )}
    </div>
  );
}

function Models({ models, total }: { models: SpendingReport["models"]; total: number }) {
  const shown = models.length > 4 ? models.slice(0, 3) : models;
  const rest = models.slice(shown.length).reduce((s, m) => s + m.total, 0);
  const parts = [...shown.map((m, i) => ({ name: m.model, total: m.total, cls: MODEL_COLORS[i] })), ...(rest > 0 ? [{ name: "Diğer", total: rest, cls: OTHER }] : [])];
  return (
    <Card className="px-4 py-4">
      <h2 className="mb-3 text-sm font-medium text-zinc-900 dark:text-zinc-100">Modeller</h2>
      <div
        className="flex h-3 gap-[2px] overflow-hidden rounded"
        role="img"
        aria-label={`Model payları: ${parts.map((p) => `${p.name} ${pct(p.total, total)}`).join(", ")}`}
      >
        {parts.map((p) => (
          <div key={p.name} className={p.cls} style={{ flexGrow: p.total, flexBasis: 0, minWidth: 2 }} title={`${p.name}: ${full.format(p.total)} token`} />
        ))}
      </div>
      <ul className="mt-3 space-y-1.5 text-sm">
        {parts.map((p) => (
          <li key={p.name} className="flex items-center gap-2">
            <span className={`h-2.5 w-2.5 shrink-0 rounded-sm ${p.cls}`} aria-hidden />
            <span className="min-w-0 flex-1 truncate text-zinc-700 dark:text-zinc-300" title={p.name}>
              {p.name}
            </span>
            <span className="tabular-nums text-zinc-900 dark:text-zinc-100">{fmt(p.total)}</span>
            <span className="w-10 text-right text-xs tabular-nums text-zinc-500 dark:text-zinc-400">{pct(p.total, total)}</span>
          </li>
        ))}
      </ul>
    </Card>
  );
}
