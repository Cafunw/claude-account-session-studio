// Dev preview only (plain `vite` in a browser, no Tauri backend). Every value here is made up.
// main.tsx imports this behind `import.meta.env.DEV`, so it never reaches the production bundle.
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { AccountProfile, EnvironmentReport, FullDashboardPayload, HealthReport, PendingChat, SessionRecord } from "./types";

const now = Date.now();
const iso = (msFromNow: number) => new Date(now + msFromNow).toISOString();
const H = 3_600_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const U1 = "11111111-aaaa-4aaa-8aaa-000000000001";
const U2 = "22222222-bbbb-4bbb-8bbb-000000000002";
const U3 = "7f3a9c21-cccc-4ccc-8ccc-000000000003";

const account = (p: Partial<AccountProfile> & Pick<AccountProfile, "accountUuid" | "email">): AccountProfile => ({
  id: p.accountUuid,
  label: p.email,
  customLabel: null,
  orgUuid: "org-" + p.accountUuid.slice(0, 8),
  planType: "Plan bilinmiyor",
  rateLimitTier: "default",
  hasDpapiCredentials: false,
  isActive: false,
  sessionCount: 0,
  healthySessionCount: 0,
  lastActivityAt: now - 2 * H,
  addedAt: iso(-30 * 24 * H),
  usage: { fiveHour: null, sevenDay: null, sevenDaySonnet: null, extraUsage: null, fetchedAt: null, source: "none", rateLimitNotice: null },
  ...p,
});

let n = 0;
const chat = (title: string, cwd: string | null, groupId: string | null = null, isStarred = false): SessionRecord => {
  n += 1;
  return {
    sessionId: `local_mock${n}`,
    fileName: `local_mock${n}.json`,
    filePath: "",
    accountUuid: U1,
    accountLabel: "",
    orgUuid: "",
    cliSessionId: n === 12 ? null : `cli-mock-${n}`,
    cwd,
    originCwd: cwd,
    title,
    createdAt: now - n * 9 * H,
    lastActivityAt: now - n * 5 * H,
    completedTurns: 4 + n,
    model: "claude-sonnet",
    isArchived: false,
    isStarred,
    transcriptUnavailable: false,
    transcriptExistsOnDisk: true,
    errorNotice: null,
    spawnedFrom: null,
    groupId,
  };
};

const state: FullDashboardPayload = {
  environment: {
    desktopRunning: true,
    activeDataRoot: { path: "C:\\Users\\ornek\\AppData\\Roaming\\Claude" },
    backupStorageDir: "C:\\Users\\ornek\\AppData\\Local\\OrnekYedekler",
  } as EnvironmentReport,
  automation: {
    autoSyncEnabled: true,
    autoSwitchEnabled: false,
    switchThresholdPercent: 90,
    pollIntervalSeconds: 60,
    syncSidebarGroups: true,
    confirmBeforeDesktopRestart: false,
    transferMode: "ask",
    selectedChats: ["cli-mock-9"],
    selectedGroups: ["grp-1"],
    selectedFolders: ["c:\\projeler\\api-servisi"],
    selectedTargets: { "group:grp-1": [U2] },
  },
  accounts: [
    account({
      accountUuid: U1,
      email: "ornek.is@example.com",
      customLabel: "İş hesabı",
      planType: "Claude Max 20x",
      hasDpapiCredentials: true,
      isActive: true,
      sessionCount: 12,
      usage: {
        fiveHour: { utilization: 34, resetsAt: iso(2.4 * H) },
        sevenDay: { utilization: 58, resetsAt: iso(3.2 * 24 * H) },
        sevenDaySonnet: { utilization: 21, resetsAt: iso(3.2 * 24 * H) },
        extraUsage: null,
        fetchedAt: iso(-4 * 60_000),
        source: "api",
        rateLimitNotice: null,
      },
    }),
    account({
      accountUuid: U2,
      email: "deneme.kisisel@example.com",
      planType: "Claude Pro",
      sessionCount: 9,
      usage: {
        fiveHour: { utilization: 76, resetsAt: iso(0.6 * H) },
        sevenDay: { utilization: 92, resetsAt: iso(1.1 * 24 * H) },
        sevenDaySonnet: null,
        extraUsage: null,
        fetchedAt: iso(-2 * H),
        source: "desktop-history",
        rateLimitNotice: "Haftalık limitin dolmak üzere.",
      },
    }),
    account({ accountUuid: U3, email: "account-7f3a9c21@claude.ai" }),
  ],
  sessions: [
    chat("Ödeme sayfası tasarımı", "C:\\Projeler\\web-sitesi", "grp-1", true),
    chat("Fatura PDF hatası", "C:\\Projeler\\web-sitesi", "grp-1"),
    chat("Müşteri raporu taslağı", null, "grp-1"),
    chat("Giriş akışını sadeleştir", "C:\\Projeler\\api-servisi"),
    chat("Rate limit middleware", "C:\\Projeler\\api-servisi", null, true),
    chat("Veritabanı göçü", "C:/Projeler/api-servisi/"),
    chat("Oyun prototipi fizik", "D:\\Oyun\\prototip"),
    chat("Seviye editörü", "D:\\Oyun\\prototip"),
    chat("Tatil planı listesi", null),
    chat("Excel formülü", null),
    chat("Notlar klasörünü düzenle", "C:\\Belgeler\\notlar"),
    chat("Eski içe aktarılmış sohbet", null),
  ],
  backups: [
    { id: "b3", createdAt: iso(-1.5 * H), reason: "Hesap değişiminden önce", sourceRoot: "", backupDir: "", totalFiles: 48, totalBytes: 0, allVerified: true, files: [] },
    { id: "b2", createdAt: iso(-26 * H), reason: "Manuel yedek", sourceRoot: "", backupDir: "", totalFiles: 45, totalBytes: 0, allVerified: true, files: [] },
    { id: "b1", createdAt: iso(-5 * 24 * H), reason: "Geri yüklemeden önce", sourceRoot: "", backupDir: "", totalFiles: 39, totalBytes: 0, allVerified: false, files: [] },
  ],
  lastSyncAt: iso(-3 * 60_000),
  sidebarGroups: [{ id: "grp-1", name: "Müşteri işleri" }],
  // Open the preview with ?uyari to see the format-warning banner.
  formatWarning: location.search.includes("uyari") ? "Örnek: beklenmeyen alan 'sessionsV3' bulundu." : null,
};

const health: HealthReport = {
  checkedChats: 30,
  checkedBackups: 3,
  chatProblems: 1,
  backupProblems: 1,
  items: [
    { kind: "chat", owner: "İş hesabı", subject: "Eski içe aktarılmış sohbet", problem: "Sohbet kaydı diskte bulunamadı." },
    { kind: "backup", owner: "Yedek", subject: "Geri yüklemeden önce", problem: "1 dosyanın özeti tutmuyor." },
  ],
};

// Chats the second account lacks: clicking "Geç" on it opens the switch dialog.
const pending: PendingChat[] = [
  { cliSessionId: "cli-mock-4", sessionId: "local_mock4", title: "Giriş akışını sadeleştir", cwd: "C:\\Projeler\\api-servisi", lastActivityAt: now - 20 * H, sourceAccountUuid: U1 },
  { cliSessionId: "cli-mock-7", sessionId: "local_mock7", title: "Oyun prototipi fizik", cwd: "D:\\Oyun\\prototip", lastActivityAt: now - 35 * H, sourceAccountUuid: U1 },
  { cliSessionId: "cli-mock-10", sessionId: "local_mock10", title: "Excel formülü", cwd: null, lastActivityAt: now - 50 * H, sourceAccountUuid: U1 },
];

const snap = () => structuredClone(state);

export function installDevMock() {
  mockIPC(
    async (cmd, args: any) => {
      switch (cmd) {
        case "get_dashboard_state":
        case "one_click_start_new_account_login":
          if (cmd !== "get_dashboard_state") await sleep(900);
          return snap();
        case "refresh_all_usage":
          await sleep(700);
          return { dashboard: snap(), notes: ["Örnek: 1 hesap için Claude Desktop kaydı kullanıldı."] };
        case "plugin:app|version":
          return "1.0.0";
        case "get_switch_plan":
          return state.automation.transferMode === "ask" && args.targetAccountUuid === U2
            ? { ask: true, pending }
            : { ask: false, pending: [] };
        case "update_preferences":
          state.automation.switchThresholdPercent = args.quotaAlertPercent;
          return snap();
        case "one_click_switch_account":
          console.info("[devMock] switch", args.targetAccountUuid, "extraChats", args.extraChats);
          await sleep(1200);
          state.accounts.forEach((a) => (a.isActive = a.accountUuid === args.targetAccountUuid));
          return snap();
        case "rename_account": {
          const a = state.accounts.find((x) => x.accountUuid === args.accountUuid);
          if (a) a.customLabel = args.label.trim() || null;
          return snap();
        }
        case "set_sync_selection":
          Object.assign(state.automation, {
            transferMode: args.mode,
            selectedChats: args.chats,
            selectedGroups: args.groups,
            selectedFolders: args.folders,
            selectedTargets: args.targets,
          });
          await sleep(200);
          return snap();
        case "create_manual_backup":
          await sleep(800);
          state.backups.unshift({ ...state.backups[0], id: `b${state.backups.length + 1}`, createdAt: iso(0), reason: args.reason });
          return null;
        case "execute_safe_restore":
          await sleep(800);
          return { createdFiles: 2 };
        case "run_health_check":
          await sleep(900);
          return health;
        default:
          console.warn("[devMock] unhandled command", cmd, args);
          return null;
      }
    },
    { shouldMockEvents: true }
  );
  // Open the preview with ?secim to see the tray's "switch-choice-needed" flow.
  if (location.search.includes("secim")) setTimeout(() => emit("switch-choice-needed", { targetAccountUuid: U2 }), 800);
}
