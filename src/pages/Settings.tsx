import { ReactNode, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { Copy, Loader2 } from "lucide-react";
import { AutomationSettings, FullDashboardPayload, UpdateInfo } from "../types";
import { installUpdate } from "../UpdateBanner";
import { Theme } from "../theme";
import { Button, Card, Segmented } from "../ui";
import { Key, pct, t } from "../i18n";

const MODES: { value: AutomationSettings["transferMode"]; title: Key; text: Key }[] = [
  { value: "ask", title: "set.modeAsk", text: "set.modeAskText" },
  { value: "all", title: "set.modeAll", text: "set.modeAllText" },
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
  onLanguage,
  onTransferMode,
  onQuotaAlert,
  onAutoUpdate,
  onNotifyOnReset,
  onSwitchShortcut,
  notify,
}: {
  data: FullDashboardPayload;
  theme: Theme;
  onTheme: (t: Theme) => void;
  onLanguage: (l: AutomationSettings["language"]) => void;
  onTransferMode: (m: AutomationSettings["transferMode"]) => void;
  onQuotaAlert: (pct: number) => void;
  onAutoUpdate: (enabled: boolean) => void;
  onNotifyOnReset: (enabled: boolean) => void;
  onSwitchShortcut: (shortcut: string) => void;
  notify: (ok: boolean, message: string) => void;
}) {
  const [version, setVersion] = useState("…");
  useEffect(() => {
    getVersion().then(setVersion, () => setVersion("—"));
  }, []);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [updateBusy, setUpdateBusy] = useState<"check" | "install" | null>(null);
  const runUpdate = async (kind: "check" | "install") => {
    setUpdateBusy(kind);
    try {
      if (kind === "check") setUpdate(await invoke<UpdateInfo>("check_for_update"));
      else await installUpdate();
    } catch (e: any) {
      notify(false, String(e));
    } finally {
      setUpdateBusy(null);
    }
  };

  const paths = [
    { label: t("set.dataFolder"), value: data.environment.activeDataRoot?.path },
    { label: t("set.backupFolder"), value: data.environment.backupStorageDir },
  ];
  const copy = (value: string) =>
    navigator.clipboard.writeText(value).then(
      () => notify(true, t("set.copied")),
      () => notify(false, t("set.copyFailed"))
    );

  return (
    <div className="max-w-3xl space-y-4">
      <Section title={t("set.appearance")}>
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm text-zinc-600 dark:text-zinc-400">{t("set.theme")}</span>
          <Segmented
            label={t("set.theme")}
            value={theme}
            onChange={onTheme}
            options={[
              { value: "system", label: t("set.themeSystem") },
              { value: "light", label: t("set.themeLight") },
              { value: "dark", label: t("set.themeDark") },
            ]}
          />
        </div>
        <div className="mt-4 flex items-center justify-between gap-4 border-t border-zinc-100 pt-4 dark:border-white/[0.06]">
          <span className="text-sm text-zinc-600 dark:text-zinc-400">{t("set.language")}</span>
          {/* Language names stay in their own language, so each is findable whatever the current one is. */}
          <Segmented
            label={t("set.language")}
            value={data.automation.language}
            onChange={onLanguage}
            options={[
              { value: "auto", label: t("set.langAuto") },
              { value: "tr", label: "Türkçe" },
              { value: "en", label: "English" },
            ]}
          />
        </div>
      </Section>

      <Section title={t("set.transfer")}>
        <div role="radiogroup" aria-label={t("set.transfer")} className="grid gap-2.5 sm:grid-cols-2">
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
                  <span className="block text-sm font-medium text-zinc-900 dark:text-zinc-100">{t(m.title)}</span>
                  <span className="mt-1 block text-pretty text-xs leading-relaxed text-zinc-600 dark:text-zinc-400">{t(m.text)}</span>
                </span>
              </label>
            );
          })}
        </div>
      </Section>

      <Section title={t("set.notifications")}>
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <p className="text-sm text-zinc-800 dark:text-zinc-200">{t("set.quotaAlert")}</p>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("set.quotaAlertText")}</p>
          </div>
          <Segmented
            label={t("set.quotaAlertLabel")}
            value={data.automation.switchThresholdPercent}
            onChange={onQuotaAlert}
            options={[70, 80, 90, 95].map((v) => ({ value: v, label: pct(v) }))}
          />
        </div>
        <div className="mt-4 flex items-center justify-between gap-4 border-t border-zinc-100 pt-4 dark:border-white/[0.06]">
          <div className="min-w-0">
            <p id="notify-reset-label" className="text-sm text-zinc-800 dark:text-zinc-200">{t("set.notifyReset")}</p>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("set.notifyResetText")}</p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={data.automation.notifyOnReset}
            aria-labelledby="notify-reset-label"
            onClick={() => onNotifyOnReset(!data.automation.notifyOnReset)}
            className={`relative h-6 w-11 shrink-0 cursor-pointer rounded-full transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-[#D97757]/50 ${
              data.automation.notifyOnReset ? "bg-[#D97757]" : "bg-zinc-300 dark:bg-white/20"
            }`}
          >
            <span
              aria-hidden
              className={`absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${
                data.automation.notifyOnReset ? "translate-x-5" : ""
              }`}
            />
          </button>
        </div>
      </Section>

      <Section title={t("set.shortcut")}>
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <p className="text-sm text-zinc-800 dark:text-zinc-200">{t("set.nextAccount")}</p>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("set.shortcutText")}</p>
          </div>
          <Segmented
            label={t("set.shortcutLabel")}
            value={data.automation.switchShortcut}
            onChange={onSwitchShortcut}
            options={[
              { value: "", label: t("common.off") },
              { value: "Ctrl+Alt+K", label: "Ctrl+Alt+K" },
              { value: "Ctrl+Alt+J", label: "Ctrl+Alt+J" },
              { value: "Ctrl+Shift+F12", label: "Ctrl+Shift+F12" },
            ]}
          />
        </div>
      </Section>

      <Section title={t("set.updates")}>
        <div className="space-y-4">
          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0">
              <p className="text-sm text-zinc-800 dark:text-zinc-200">{t("set.autoUpdate")}</p>
              <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("set.autoUpdateText")}</p>
            </div>
            <Segmented
              label={t("set.autoUpdate")}
              value={data.automation.autoUpdate}
              onChange={onAutoUpdate}
              options={[
                { value: true, label: t("common.on") },
                { value: false, label: t("common.off") },
              ]}
            />
          </div>
          <div className="flex items-center justify-between gap-4 border-t border-zinc-100 pt-4 dark:border-white/[0.06]">
            <div className="min-w-0 text-sm" aria-live="polite">
              <p className="text-zinc-800 dark:text-zinc-200">
                {t("set.installed")} <span className="font-medium tabular-nums">v{version}</span>
              </p>
              {update && (
                <p className={`text-xs ${update.available ? "font-medium text-[#C2603F] dark:text-[#E89A7E]" : "text-zinc-500 dark:text-zinc-400"}`}>
                  {update.available ? t("set.available", { version: update.version ?? "" }) : t("set.upToDate")}
                </p>
              )}
            </div>
            <div className="flex shrink-0 gap-2">
              {update?.available && (
                <Button variant="primary" onClick={() => runUpdate("install")} disabled={!!updateBusy}>
                  {updateBusy === "install" && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
                  {updateBusy === "install" ? t("common.updating") : t("common.updateNow")}
                </Button>
              )}
              <Button onClick={() => runUpdate("check")} disabled={!!updateBusy}>
                {updateBusy === "check" && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
                {updateBusy === "check" ? t("set.checking") : t("set.checkNow")}
              </Button>
            </div>
          </div>
        </div>
      </Section>

      <Section title={t("set.about")}>
        <dl className="space-y-3 text-sm">
          <div className="flex items-center justify-between gap-4">
            <dt className="text-zinc-600 dark:text-zinc-400">{t("set.version")}</dt>
            <dd className="font-medium tabular-nums text-zinc-900 dark:text-zinc-100">{version}</dd>
          </div>
          {paths.map((p) => (
            <div key={p.label} className="flex items-center gap-3">
              <div className="min-w-0 flex-1">
                <dt className="text-zinc-600 dark:text-zinc-400">{p.label}</dt>
                <dd className="truncate font-mono text-xs text-zinc-800 dark:text-zinc-200" title={p.value ?? undefined}>
                  {p.value || t("set.notFound")}
                </dd>
              </div>
              {p.value && (
                <Button variant="ghost" className="h-8 px-2.5 text-xs" onClick={() => copy(p.value!)} aria-label={t("set.copyPath", { label: p.label })}>
                  <Copy className="h-3.5 w-3.5" aria-hidden />
                  {t("set.copy")}
                </Button>
              )}
            </div>
          ))}
        </dl>
      </Section>
    </div>
  );
}
