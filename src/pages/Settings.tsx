import { ReactNode, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Copy } from "lucide-react";
import { AutomationSettings, FullDashboardPayload } from "../types";
import { Theme } from "../theme";
import { Button, Card, Segmented } from "../ui";

const MODES: { value: AutomationSettings["transferMode"]; title: string; text: string }[] = [
  {
    value: "ask",
    title: "Yeni sohbetleri geçişte sor",
    text: "Hesap değiştirirken diğer hesapta olmayan sohbetleri sorar; daha önce aktarılanlar kendiliğinden eşitlenir.",
  },
  { value: "all", title: "Hepsini otomatik aktar", text: "Tüm sohbetler sormadan bütün hesaplara aktarılır." },
];

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card className="p-5">
      <h2 className="mb-3 text-[15px] font-semibold text-zinc-900 dark:text-zinc-100">{title}</h2>
      {children}
    </Card>
  );
}

export default function SettingsPage({
  data,
  theme,
  onTheme,
  onTransferMode,
  onQuotaAlert,
  notify,
}: {
  data: FullDashboardPayload;
  theme: Theme;
  onTheme: (t: Theme) => void;
  onTransferMode: (m: AutomationSettings["transferMode"]) => void;
  onQuotaAlert: (pct: number) => void;
  notify: (ok: boolean, message: string) => void;
}) {
  const [version, setVersion] = useState("…");
  useEffect(() => {
    getVersion().then(setVersion, () => setVersion("—"));
  }, []);

  const paths = [
    { label: "Claude Desktop veri klasörü", value: data.environment.activeDataRoot?.path },
    { label: "Yedek klasörü", value: data.environment.backupStorageDir },
  ];
  const copy = (value: string) =>
    navigator.clipboard.writeText(value).then(
      () => notify(true, "Yol kopyalandı."),
      () => notify(false, "Kopyalanamadı.")
    );

  return (
    <div className="max-w-3xl space-y-4">
      <Section title="Görünüm">
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm text-zinc-600 dark:text-zinc-400">Tema</span>
          <Segmented
            label="Tema"
            value={theme}
            onChange={onTheme}
            options={[
              { value: "system", label: "Sistem" },
              { value: "light", label: "Açık" },
              { value: "dark", label: "Koyu" },
            ]}
          />
        </div>
      </Section>

      <Section title="Sohbet aktarımı">
        <div role="radiogroup" aria-label="Sohbet aktarımı" className="grid gap-2.5 sm:grid-cols-2">
          {MODES.map((m) => {
            const on = data.automation.transferMode === m.value;
            return (
              <label
                key={m.value}
                className={`flex cursor-pointer gap-3 rounded-xl border p-4 transition has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-[#D97757]/70 ${
                  on
                    ? "border-[#D97757]/60 bg-[#D97757]/[0.06] dark:border-[#D97757]/50 dark:bg-[#D97757]/10"
                    : "border-zinc-200 hover:bg-zinc-50 dark:border-white/10 dark:hover:bg-white/[0.04]"
                }`}
              >
                <input
                  type="radio"
                  name="transferMode"
                  checked={on}
                  onChange={() => onTransferMode(m.value)}
                  className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-[#D97757] focus:outline-none"
                />
                <span className="min-w-0">
                  <span className="block text-sm font-medium text-zinc-900 dark:text-zinc-100">{m.title}</span>
                  <span className="mt-1 block text-pretty text-xs leading-relaxed text-zinc-600 dark:text-zinc-400">{m.text}</span>
                </span>
              </label>
            );
          })}
        </div>
      </Section>

      <Section title="Bildirimler">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <p className="text-sm text-zinc-800 dark:text-zinc-200">Kota uyarısı</p>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">Açık hesabın kotası bu orana gelince haber verilir.</p>
          </div>
          <Segmented
            label="Kota uyarısı eşiği"
            value={data.automation.switchThresholdPercent}
            onChange={onQuotaAlert}
            options={[70, 80, 90, 95].map((v) => ({ value: v, label: `%${v}` }))}
          />
        </div>
      </Section>

      <Section title="Hakkında">
        <dl className="space-y-3 text-sm">
          <div className="flex items-center justify-between gap-4">
            <dt className="text-zinc-600 dark:text-zinc-400">Sürüm</dt>
            <dd className="font-medium tabular-nums text-zinc-900 dark:text-zinc-100">{version}</dd>
          </div>
          {paths.map((p) => (
            <div key={p.label} className="flex items-center gap-3">
              <div className="min-w-0 flex-1">
                <dt className="text-zinc-600 dark:text-zinc-400">{p.label}</dt>
                <dd className="truncate font-mono text-xs text-zinc-800 dark:text-zinc-200" title={p.value ?? undefined}>
                  {p.value || "bulunamadı"}
                </dd>
              </div>
              {p.value && (
                <Button variant="ghost" className="h-8 px-2.5 text-xs" onClick={() => copy(p.value!)} aria-label={`${p.label} yolunu kopyala`}>
                  <Copy className="h-3.5 w-3.5" aria-hidden />
                  Kopyala
                </Button>
              )}
            </div>
          ))}
        </dl>
      </Section>
    </div>
  );
}
