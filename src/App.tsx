import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  ChartLine,
  ChevronLeft,
  ChevronRight,
  Coins,
  HeartPulse,
  Loader2,
  MessagesSquare,
  ScrollText,
  Settings,
  ShieldAlert,
  UserRoundCog,
  Users,
} from "lucide-react";
import { AccountProfile, AutomationSettings, FullDashboardPayload, HealthReport, PendingChat, SwitchPlan } from "./types";
import { accountName, formatAgo, focusRing } from "./ui";
import AccountsPage from "./pages/Accounts";
import ChatsPage, { Selection } from "./pages/Chats";
import BackupsPage from "./pages/Backups";
import HealthPage from "./pages/Health";
import JournalPage from "./pages/Journal";
import SettingsPage from "./pages/Settings";
import UsageHistoryPage from "./pages/UsageHistory";
import SpendingPage from "./pages/Spending";
import SwitchDialog from "./SwitchDialog";
import UpdateBanner from "./UpdateBanner";
import { useTheme } from "./theme";

const PAGES = [
  { id: "accounts", label: "Hesaplar", icon: Users, subtitle: "Claude hesaplarını yönet" },
  { id: "history", label: "Kota geçmişi", icon: ChartLine, subtitle: "Hesapların kota kullanımı ve haftalık limit tahmini" },
  { id: "spending", label: "Harcama", icon: Coins, subtitle: "Hangi proje ve sohbetlerin en çok token kullandığı" },
  { id: "chats", label: "Sohbetler", icon: MessagesSquare, subtitle: "Sohbetlerin ve diğer hesaplara aktarımı" },
  { id: "backups", label: "Yedekler", icon: Archive, subtitle: "Sohbet yedeklerini gör ve eksikleri geri yükle" },
  { id: "health", label: "Sağlık", icon: HeartPulse, subtitle: "Sohbetleri ve yedekleri salt okunur kontrol et" },
  { id: "journal", label: "İşlem günlüğü", icon: ScrollText, subtitle: "Son işlemleri ve hataları salt okunur gör" },
  { id: "settings", label: "Ayarlar", icon: Settings, subtitle: "Görünüm, sohbet aktarımı ve bildirimler" },
] as const;
type PageId = (typeof PAGES)[number]["id"];

export default function App() {
  const [data, setData] = useState<FullDashboardPayload | null>(null);
  const [progressStep, setProgressStep] = useState<string | null>(null);
  const [toast, setToast] = useState<{ type: "success" | "error"; message: string; action?: { label: string; run: () => void } } | null>(null);
  const [health, setHealth] = useState<HealthReport | null>(null);
  const [healthBusy, setHealthBusy] = useState(false);
  const [usageBusy, setUsageBusy] = useState(false);
  const [page, setPage] = useState<PageId>("accounts");
  const [collapsed, setCollapsed] = useState(false);
  const [theme, setTheme] = useTheme();
  const [plan, setPlan] = useState<{ acc: AccountProfile; pending: PendingChat[] } | null>(null);
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);

  const fetchState = async () => {
    try {
      setData(await invoke<FullDashboardPayload>("get_dashboard_state"));
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  useEffect(() => {
    fetchState();
    const poll = setInterval(fetchState, 60_000);
    const unlistens = [
      listen<string>("switch-progress", (event) => setProgressStep(event.payload)),
      // Only the tray switch emits this; the button flow reports through invoke's result.
      listen<{ ok: boolean; message: string }>("switch-finished", (event) => {
        setProgressStep(null);
        fetchState();
        setToast({ type: event.payload.ok ? "success" : "error", message: event.payload.message });
      }),
      // A tray switch that needs a choice; the window is already shown.
      listen<{ targetAccountUuid: string }>("switch-choice-needed", async (event) => {
        try {
          const d = await invoke<FullDashboardPayload>("get_dashboard_state");
          setData(d);
          const acc = d.accounts.find((a) => a.accountUuid === event.payload.targetAccountUuid);
          if (acc) handleSwitch(acc);
        } catch (e: any) {
          setToast({ type: "error", message: String(e) });
        }
      }),
      listen("sessions-synced", () => fetchState()),
      listen<{ version: string }>("update-available", (event) => setUpdateVersion(event.payload.version)),
      listen("account-detected", () => {
        fetchState();
        setToast({
          type: "success",
          message: "Yeni hesap algılandı. Claude Desktop kapandığında girişi otomatik kaydedilecek.",
        });
      }),
      listen("account-saved", () => {
        fetchState();
        setToast({ type: "success", message: "Yeni hesabın girişi kaydedildi." });
      }),
    ];
    return () => {
      clearInterval(poll);
      unlistens.forEach((u) => u.then((fn) => fn()));
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 6000);
    return () => clearTimeout(t);
  }, [toast]);

  const handleExport = async (cliSessionId: string) => {
    try {
      const path = await invoke<string>("export_chat_markdown", { cliSessionId });
      setToast({
        type: "success",
        message: `Kaydedildi: ${path.split(/[\\/]/).pop()}`,
        action: {
          label: "Klasörde göster",
          run: () => invoke("reveal_in_explorer", { path }).catch((e) => setToast({ type: "error", message: String(e) })),
        },
      });
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  // Runs a long backend action behind the progress overlay.
  const runAction = async (firstStep: string, action: () => Promise<void>) => {
    setProgressStep(firstStep);
    try {
      await action();
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    } finally {
      setProgressStep(null);
    }
  };

  const handleRefreshUsage = async () => {
    setUsageBusy(true);
    try {
      const r = await invoke<{ dashboard: FullDashboardPayload; notes: string[] }>("refresh_all_usage");
      setData(r.dashboard);
      setToast({ type: "success", message: ["Limitler yenilendi", ...r.notes].join("\n") });
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    } finally {
      setUsageBusy(false);
    }
  };

  const doSwitch = (acc: AccountProfile, extraChats: string[]) =>
    runAction("Claude Desktop kapatılıyor...", async () => {
      setData(await invoke<FullDashboardPayload>("one_click_switch_account", { targetAccountUuid: acc.accountUuid, extraChats }));
      setToast({ type: "success", message: `${accountName(acc).name} hesabına geçildi. Sohbetlerin hazır.` });
    });

  // Uses only setters, so the first render's copy held by the event listener stays correct.
  const handleSwitch = async (acc: AccountProfile) => {
    try {
      const p = await invoke<SwitchPlan>("get_switch_plan", { targetAccountUuid: acc.accountUuid });
      if (p.ask && p.pending.length > 0) setPlan({ acc, pending: p.pending });
      else doSwitch(acc, []);
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  const handleRename = async (acc: AccountProfile, label: string) => {
    try {
      setData(await invoke<FullDashboardPayload>("rename_account", { accountUuid: acc.accountUuid, label }));
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  const handleMove = async (acc: AccountProfile, direction: "up" | "down") => {
    try {
      setData(await invoke<FullDashboardPayload>("move_account", { accountUuid: acc.accountUuid, direction }));
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  const handleRemove = async (acc: AccountProfile) => {
    try {
      setData(await invoke<FullDashboardPayload>("remove_account", { accountUuid: acc.accountUuid }));
      setToast({ type: "success", message: `${accountName(acc).name} uygulamadan kaldırıldı. Sohbetlerin yerinde.` });
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  const handleAddAccount = () =>
    runAction("Mevcut hesap kaydediliyor...", async () => {
      setData(await invoke<FullDashboardPayload>("one_click_start_new_account_login"));
      setToast({ type: "success", message: "Claude Desktop'ta yeni hesapla giriş yap. Gerisi otomatik." });
    });

  const saveSelection = async (patch: Partial<Selection>) => {
    if (!data) return;
    const a = { ...data.automation, ...patch };
    // Show the tick at once; the reply rescans every chat and can take a moment.
    setData((d) => d && { ...d, automation: { ...d.automation, ...patch } });
    try {
      setData(
        await invoke<FullDashboardPayload>("set_sync_selection", {
          mode: a.transferMode,
          chats: a.selectedChats,
          groups: a.selectedGroups,
          folders: a.selectedFolders,
          targets: a.selectedTargets,
        })
      );
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  const handleQuotaAlert = async (quotaAlertPercent: number) => {
    try {
      setData(await invoke<FullDashboardPayload>("update_preferences", { quotaAlertPercent }));
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  const handleNotifyOnReset = async (enabled: boolean) => {
    try {
      setData(await invoke<FullDashboardPayload>("set_notify_on_reset", { enabled }));
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  // Registration can fail (taken by another app); the backend's message goes to the toast as is.
  const handleSwitchShortcut = async (shortcut: string) => {
    try {
      setData(await invoke<FullDashboardPayload>("set_switch_shortcut", { shortcut }));
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    }
  };

  const handleRestoreBackup = (backupId: string) =>
    runAction("Yedek doğrulanıyor...", async () => {
      const res = await invoke<any>("execute_safe_restore", { backupId });
      await fetchState();
      setToast({
        type: "success",
        message: `${res.createdFiles} eksik sohbet geri yüklendi. Mevcut sohbetlere dokunulmadı.`,
      });
    });

  const handleManualBackup = () =>
    runAction("Yedek alınıyor...", async () => {
      await invoke("create_manual_backup", { reason: "Manuel yedek" });
      await fetchState();
      setToast({ type: "success", message: "Yedek alındı ve doğrulandı." });
    });

  const handleHealthCheck = async () => {
    setHealthBusy(true);
    try {
      setHealth(await invoke<HealthReport>("run_health_check"));
    } catch (e: any) {
      setToast({ type: "error", message: String(e) });
    } finally {
      setHealthBusy(false);
    }
  };

  const notify = useCallback((ok: boolean, message: string) => setToast({ type: ok ? "success" : "error", message }), []);
  const current = PAGES.find((p) => p.id === page)!;
  const accounts = data ? data.accounts : []; // already in the user's stored order
  const counts: Partial<Record<PageId, number>> = data ? { chats: data.sessions.length, backups: data.backups.length } : {};
  const running = !!data?.environment.desktopRunning;

  return (
    <div className="flex h-[100dvh] overflow-hidden bg-[#F6F6F7] dark:bg-[#111214] text-zinc-900 dark:text-zinc-100">
      <aside className={`flex shrink-0 flex-col border-r border-zinc-200 dark:border-white/10 bg-white dark:bg-[#17181c] transition-[width] ${collapsed ? "w-16" : "w-56"}`}>
        <div className={`flex items-center gap-3 py-5 ${collapsed ? "justify-center" : "px-5"}`}>
          <span className="relative flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-zinc-200 dark:border-white/10 bg-zinc-50 dark:bg-white/[0.03] text-zinc-800 dark:text-zinc-200">
            <UserRoundCog className="h-5 w-5" aria-hidden />
            <span className="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-[#D97757] ring-2 ring-white dark:ring-[#17181c]" aria-hidden />
          </span>
          {!collapsed && (
            <div className="min-w-0 leading-tight">
              <p className="truncate text-[15px] font-semibold">Claude Hesap</p>
              <p className="truncate text-[13px] text-zinc-500 dark:text-zinc-400">Yöneticisi</p>
            </div>
          )}
        </div>

        <nav aria-label="Sayfalar" className="flex-1 space-y-1 px-2.5">
          {PAGES.map((p) => {
            const on = p.id === page;
            return (
              <button
                key={p.id}
                type="button"
                onClick={() => setPage(p.id)}
                aria-current={on ? "page" : undefined}
                aria-label={collapsed ? p.label : undefined}
                title={collapsed ? p.label : undefined}
                className={`flex w-full items-center gap-3 rounded-lg py-2.5 text-[15px] transition ${focusRing} ${
                  collapsed ? "justify-center" : "px-3"
                } ${on ? "bg-sky-50 dark:bg-sky-400/10 font-medium text-sky-900 dark:text-sky-100 ring-1 ring-inset ring-sky-200 dark:ring-sky-400/25" : "text-zinc-600 dark:text-zinc-400 hover:bg-zinc-50 dark:hover:bg-white/[0.05] hover:text-zinc-900 dark:hover:text-zinc-100"}`}
              >
                <p.icon className={`h-[18px] w-[18px] shrink-0 ${on ? "text-sky-700 dark:text-sky-300" : "text-zinc-500 dark:text-zinc-400"}`} aria-hidden />
                {!collapsed && (
                  <>
                    <span className="flex-1 text-left">{p.label}</span>
                    {counts[p.id] !== undefined && <span className="text-xs tabular-nums text-zinc-400">{counts[p.id]}</span>}
                  </>
                )}
              </button>
            );
          })}
        </nav>

        <div className="border-t border-zinc-100 dark:border-white/[0.06] px-2.5 py-3">
          <button
            type="button"
            onClick={() => setCollapsed((c) => !c)}
            aria-expanded={!collapsed}
            aria-label={collapsed ? "Kenar çubuğunu genişlet" : "Kenar çubuğunu daralt"}
            title={collapsed ? "Kenar çubuğunu genişlet" : undefined}
            className={`flex w-full items-center gap-2 rounded-lg py-2 text-[13px] text-zinc-600 dark:text-zinc-400 transition hover:bg-zinc-50 dark:hover:bg-white/[0.05] ${focusRing} ${
              collapsed ? "justify-center" : "px-3"
            }`}
          >
            {collapsed ? <ChevronRight className="h-4 w-4" aria-hidden /> : <ChevronLeft className="h-4 w-4" aria-hidden />}
            {!collapsed && "Kenar çubuğunu daralt"}
          </button>
        </div>
        <div
          className={`flex items-center gap-3 border-t border-zinc-100 dark:border-white/[0.06] py-3.5 ${collapsed ? "justify-center" : "px-5"}`}
          title={collapsed ? `Claude Desktop ${running ? "açık" : "kapalı"}` : undefined}
        >
          <span className={`h-2 w-2 shrink-0 rounded-full ${running ? "bg-emerald-500" : "bg-zinc-300 dark:bg-zinc-600"}`} aria-hidden />
          {!collapsed && (
            <div className="min-w-0 leading-tight">
              <p className="truncate text-[13px] font-medium text-zinc-800 dark:text-zinc-200">Claude Desktop {running ? "açık" : "kapalı"}</p>
              <p className="truncate text-xs text-zinc-500 dark:text-zinc-400">son eşitleme: {data ? formatAgo(data.lastSyncAt) : "…"}</p>
            </div>
          )}
        </div>
      </aside>

      {/* key: a new page starts scrolled to the top */}
      <main key={page} className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-6xl px-8 pb-12 pt-7">
          {data?.formatWarning && (
            <div role="alert" className="mb-6 flex gap-3 rounded-xl border border-rose-200 dark:border-rose-400/25 bg-rose-50 dark:bg-rose-500/10 px-4 py-3">
              <ShieldAlert className="mt-0.5 h-5 w-5 shrink-0 text-rose-600 dark:text-rose-400" aria-hidden />
              <div className="min-w-0">
                <p className="text-sm font-medium text-rose-900 dark:text-rose-200">
                  Claude Desktop'un dosya yapısı değişmiş görünüyor; sohbetlere dokunmamak için aktarım durduruldu.
                </p>
                <p className="mt-1 text-xs text-rose-700 dark:text-rose-300">{data.formatWarning}</p>
              </div>
            </div>
          )}

          {updateVersion && (
            <UpdateBanner
              version={updateVersion}
              onDismiss={() => setUpdateVersion(null)}
              onError={(message) => setToast({ type: "error", message })}
            />
          )}

          <header className="mb-6 flex items-center gap-4">
            <span className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border border-sky-200 dark:border-sky-400/25 bg-sky-50 dark:bg-sky-400/10 text-sky-700 dark:text-sky-300">
              <current.icon className="h-6 w-6" aria-hidden />
            </span>
            <div className="min-w-0">
              <h1 className="text-[26px] font-semibold leading-tight tracking-tight">{current.label}</h1>
              <p className="text-[15px] text-zinc-500 dark:text-zinc-400">{current.subtitle}</p>
            </div>
          </header>

          {!data ? (
            <div className="space-y-3" aria-busy="true">
              <div className="grid grid-cols-4 gap-3">
                {[0, 1, 2, 3].map((i) => (
                  <div key={i} className="h-[84px] animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
                ))}
              </div>
              <div className="h-28 animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
              <div className="h-28 animate-pulse rounded-xl bg-zinc-200/60 dark:bg-white/[0.06]" />
            </div>
          ) : page === "accounts" ? (
            <AccountsPage
              accounts={accounts}
              busy={!!progressStep}
              usageBusy={usageBusy}
              selecting={data.automation.transferMode === "ask"}
              onRefresh={handleRefreshUsage}
              onAdd={handleAddAccount}
              onSwitch={handleSwitch}
              onRename={handleRename}
              onMove={handleMove}
              onRemove={handleRemove}
            />
          ) : page === "history" ? (
            <UsageHistoryPage accounts={accounts} threshold={data.automation.switchThresholdPercent} />
          ) : page === "spending" ? (
            <SpendingPage />
          ) : page === "chats" ? (
            <ChatsPage data={data} accounts={accounts} saveSelection={saveSelection} onOpenSettings={() => setPage("settings")} onExport={handleExport} />
          ) : page === "backups" ? (
            <BackupsPage backups={data.backups} busy={!!progressStep} onBackup={handleManualBackup} onRestore={handleRestoreBackup} />
          ) : page === "health" ? (
            <HealthPage health={health} busy={healthBusy} onCheck={handleHealthCheck} />
          ) : page === "journal" ? (
            <JournalPage notify={notify} />
          ) : (
            <SettingsPage
              data={data}
              theme={theme}
              onTheme={setTheme}
              onTransferMode={(transferMode: AutomationSettings["transferMode"]) => saveSelection({ transferMode })}
              onQuotaAlert={handleQuotaAlert}
              onAutoUpdate={async (enabled) => {
                try {
                  setData(await invoke<FullDashboardPayload>("set_auto_update", { enabled }));
                } catch (e: any) {
                  setToast({ type: "error", message: String(e) });
                }
              }}
              onNotifyOnReset={handleNotifyOnReset}
              onSwitchShortcut={handleSwitchShortcut}
              notify={notify}
            />
          )}
        </div>
      </main>

      {plan && (
        <SwitchDialog
          targetName={accountName(plan.acc).name}
          pending={plan.pending}
          onCancel={() => setPlan(null)}
          onConfirm={(extraChats) => {
            setPlan(null);
            doSwitch(plan.acc, extraChats);
          }}
        />
      )}

      {progressStep && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-zinc-900/25 dark:bg-black/50 p-6 backdrop-blur-[2px]">
          <div className="w-full max-w-sm rounded-2xl border border-zinc-200 dark:border-white/10 bg-white dark:bg-[#17181c] p-6 text-center shadow-xl" role="status">
            <Loader2 className="mx-auto h-6 w-6 animate-spin text-[#D97757]" aria-hidden />
            <p className="mt-4 text-sm font-medium text-zinc-900 dark:text-zinc-100">{progressStep}</p>
            <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">Claude Desktop birazdan kendiliğinden açılacak.</p>
          </div>
        </div>
      )}

      {toast && (
        <div className="fixed bottom-5 right-5 z-40 max-w-md" role="status">
          <button
            type="button"
            onClick={() => setToast(null)}
            className={`w-full whitespace-pre-line rounded-xl border px-4 py-3 text-left text-sm shadow-lg ${focusRing} ${
              toast.type === "success" ? "border-zinc-200 dark:border-white/10 bg-white dark:bg-[#17181c] text-zinc-800 dark:text-zinc-200" : "border-rose-200 dark:border-rose-400/25 bg-rose-50 dark:bg-rose-500/10 text-rose-900 dark:text-rose-200"
            }`}
          >
            {toast.message}
          </button>
          {toast.action && (
            <button
              type="button"
              onClick={toast.action.run}
              className={`mt-1.5 w-full rounded-xl border border-zinc-200 bg-white px-4 py-2 text-left text-sm font-medium text-[#A8492A] shadow-lg dark:border-white/10 dark:bg-[#17181c] dark:text-[#EBA488] ${focusRing}`}
            >
              {toast.action.label}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
