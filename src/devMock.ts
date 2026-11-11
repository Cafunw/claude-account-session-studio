// Dev preview only (plain `vite` in a browser, no Tauri backend). Every value here is made up.
// main.tsx imports this behind `import.meta.env.DEV`, so it never reaches the production bundle.
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { AccountProfile, EnvironmentReport, FullDashboardPayload, HealthReport, PendingChat, SessionRecord, SpendingReport, TokenSums, UsageHistory } from "./types";

const now = Date.now();
const iso = (msFromNow: number) => new Date(now + msFromNow).toISOString();
const H = 3_600_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
// Strings the real backend words in the UI language ("auto" here follows the browser, as Windows does there).
const eff = (): "tr" | "en" =>
  state.automation.language === "auto" ? (navigator.language.toLowerCase().startsWith("tr") ? "tr" : "en") : state.automation.language;
const L = (tr: string, en: string) => (eff() === "tr" ? tr : en);

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
    notifyOnReset: true,
    switchShortcut: "",
    pollIntervalSeconds: 60,
    syncSidebarGroups: true,
    confirmBeforeDesktopRestart: false,
    transferMode: "ask",
    autoUpdate: true,
    selectedChats: ["cli-mock-9"],
    selectedGroups: ["grp-1"],
    selectedFolders: ["c:\\projeler\\api-servisi"],
    selectedTargets: { "group:grp-1": [U2] },
    hiddenAccounts: [],
    accountOrder: [],
    language: "auto",
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
        rateLimitNotice: "Rate limited",
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
    // Reasons as the backend really stores them (a mix of English and Turkish).
    { id: "b3", createdAt: iso(-1.5 * H), reason: "Pre-sync verified snapshot (2 new, 0 conflict-companions, 0 metadata refreshes)", sourceRoot: "", backupDir: "", totalFiles: 48, totalBytes: 0, allVerified: true, files: [] },
    { id: "b2", createdAt: iso(-26 * H), reason: "Manuel yedek", sourceRoot: "", backupDir: "", totalFiles: 45, totalBytes: 0, allVerified: true, files: [] },
    { id: "b1", createdAt: iso(-5 * 24 * H), reason: "Yeni hesap ekleme öncesi otomatik yedek", sourceRoot: "", backupDir: "", totalFiles: 39, totalBytes: 0, allVerified: false, files: [] },
  ],
  lastSyncAt: iso(-3 * 60_000),
  sidebarGroups: [{ id: "grp-1", name: "Müşteri işleri" }],
  // Open the preview with ?uyari to see the format-warning banner.
  formatWarning: null, // set per language in snap()
  effectiveLanguage: "tr",
};

const health = (): HealthReport => ({
  checkedChats: 30,
  checkedBackups: 3,
  chatProblems: 1,
  backupProblems: 1,
  items: [
    { kind: "chat", owner: "İş hesabı", subject: "Eski içe aktarılmış sohbet", problem: L("Sohbetin kayıt dosyası (.jsonl) diskte bulunamadı.", "The chat's transcript file (.jsonl) is missing from disk.") },
    { kind: "backup", owner: "Yeni hesap ekleme öncesi otomatik yedek", subject: "backup-20250110-093000-c3d4e5", problem: L("Yedek doğrulaması başarısız: SHA-256 tutmuyor", "Backup verification failed: SHA-256 mismatch") },
  ],
});

// Chats the second account lacks: clicking "Geç" on it opens the switch dialog.
const pending: PendingChat[] = [
  { cliSessionId: "cli-mock-4", sessionId: "local_mock4", title: "Giriş akışını sadeleştir", cwd: "C:\\Projeler\\api-servisi", lastActivityAt: now - 20 * H, sourceAccountUuid: U1 },
  { cliSessionId: "cli-mock-7", sessionId: "local_mock7", title: "Oyun prototipi fizik", cwd: "D:\\Oyun\\prototip", lastActivityAt: now - 35 * H, sourceAccountUuid: U1 },
  { cliSessionId: "cli-mock-10", sessionId: "local_mock10", title: "Excel formülü", cwd: null, lastActivityAt: now - 50 * H, sourceAccountUuid: U1 },
];

const snap = () => {
  const s = structuredClone(state);
  s.effectiveLanguage = eff();
  // Open the preview with ?uyari to see the format-warning banner.
  if (location.search.includes("uyari"))
    s.formatWarning = L("Örnek: En yeni 5 sohbet dosyasının hiçbirinde beklenen alan yok ya da türü değişmiş: sessionId.", "Example: None of the 5 newest chat files has the expected fields, or their type changed: sessionId.");
  return s;
};

// 14 days of hourly-ish samples: a weekly sawtooth that resets once, a 5-hour wave; U3 has none.
function history(uuid: string, days: number): UsageHistory {
  if (uuid === U3) return { samples: [], resets: [], forecast: L("Tahmin için yeterli veri yok", "Not enough data for a forecast") };
  const pro = uuid === U2;
  const reset = now - (pro ? 5.9 : 3.8) * 24 * H;
  const week = 7 * 24 * H;
  const start = now - 14 * 24 * H;
  const samples = [];
  for (let t = start; t <= now; t += 1.5 * H) {
    const since = ((((t - reset) % week) + week) % week) / (24 * H);
    const sd = Math.min(100, since * (pro ? 15.5 : 14.5) * (t < reset ? 0.9 : 1) + 2 * Math.sin(t / (5 * H)));
    const fh = pro ? null : Math.max(0, 40 * Math.sin((t / (5 * H)) % Math.PI));
    samples.push({ t, accountUuid: uuid, sd: Math.max(0, sd), fh, src: "oauth-api" });
  }
  return {
    samples: samples.filter((s) => s.t >= now - days * 24 * H),
    resets: [reset, reset - week].filter((r) => r > start && r >= now - days * 24 * H),
    forecast: pro
      ? L("Bu hızla cuma 14:00 civarı dolar", "At this pace it fills up around Friday 14:00")
      : L("Bu hızla sıfırlanmadan önce dolmaz", "At this pace it won't fill up before the reset"),
  };
}

const journal = [
  { timestamp: iso(-3 * 60_000), operation: "one-click-switch", mode: "live", details: "ornek@example.com hesabına geçildi; 2 sohbet eklendi.", status: "ok" },
  { timestamp: iso(-40 * 60_000), operation: "auto-sync", mode: "live", details: "Dosya izleme: Claude Desktop veri klasörüne erişilemedi.", status: "error" },
  { timestamp: iso(-2 * H), operation: "session-sync", mode: "live", details: "3 sohbet işaretçisi 1 hesaba kopyalandı.", status: "ok" },
  { timestamp: iso(-2 * H), operation: "backup", mode: "pre-sync", details: "backup-20250115-101500-a1b2c3: 42 dosya, doğrulandı.", status: "ok" },
  { timestamp: iso(-5 * H), operation: "prune", mode: "auto", details: "Deleted old backup backup-20241201-090000-d4e5f6", status: "ok" },
  { timestamp: iso(-9 * H), operation: "format-guard", mode: "live", details: "Sohbet dosyalarının biçimi beklenenden farklı; yazma durduruldu.", status: "error" },
  { timestamp: iso(-26 * H), operation: "update", mode: "check", details: "Güncelleme sunucusuna ulaşılamadı.", status: "error" },
  { timestamp: iso(-30 * H), operation: "remove-account", mode: "live", details: "Hesap uygulamadan kaldırıldı (sohbetlere dokunulmadı).", status: "ok" },
  { timestamp: iso(-50 * H), operation: "restore", mode: "safe", details: "2 eksik sohbet geri yüklendi.", status: "ok" },
  { timestamp: iso(-72 * H), operation: "auto-capture-account", mode: "live", details: "Yeni hesabın girişi kaydedildi.", status: "ok" },
  { timestamp: iso(-80 * H), operation: "prepare-new-account-login", mode: "live", details: "Yeni hesap girişi hazırlandı.", status: "ok" },
];

export function installDevMock() {
  mockIPC(
    async (cmd, args: any) => {
      switch (cmd) {
        case "get_dashboard_state":
        case "one_click_start_new_account_login":
          if (cmd !== "get_dashboard_state") await sleep(900);
          return snap();
        case "get_usage_history":
          await sleep(150);
          return history(args.accountUuid, args.days);
        case "get_spending":
          // First open of each range acts like a cold scan with progress events; later ones are warm.
          // A second request joins the running scan, as the backend does.
          if (!scans.has(args.days)) {
            const total = args.days > 7 ? 2686 : 1406;
            const slow = location.search.includes("yavas");
            scans.set(args.days, (async () => {
              for (let i = 0; i <= 20; i++) {
                await emit("spending-progress", { done: Math.round((total * i) / 20), total });
                await sleep(slow ? 1500 : 120);
              }
            })());
          } else await sleep(400);
          await scans.get(args.days);
          return location.search.includes("bos") ? { ...spending(args.days), totals: { tokens: { input: 0, output: 0, cacheWrite: 0, cacheRead: 0, total: 0, work: 0 }, messages: 0, chats: 0 }, projects: [], chats: [], models: [] } : spending(args.days);
        case "refresh_all_usage":
          await sleep(700);
          return {
            dashboard: snap(),
            notes: [L("deneme.kisisel@example.com: anlık veri yok, Claude Desktop'un 2 saat önceki kaydı gösteriliyor", "deneme.kisisel@example.com: no live data, showing Claude Desktop's record from 2 hours ago")],
          };
        case "plugin:app|version":
          return "1.0.0";
        case "get_switch_plan":
          return state.automation.transferMode === "ask" && args.targetAccountUuid === U2
            ? { ask: true, pending }
            : { ask: false, pending: [] };
        case "check_for_update":
          await sleep(700);
          return { available: true, version: "1.1.0", notes: "Örnek sürüm notu.", currentVersion: "1.0.0" };
        case "install_update":
          await sleep(1200);
          throw L("Güncelleme kurulamadı: önizlemede kurulamaz", "Couldn't install the update: not possible in the preview");
        case "set_language":
          state.automation.language = args.language;
          return snap();
        case "set_auto_update":
          state.automation.autoUpdate = args.enabled;
          return snap();
        case "update_preferences":
          state.automation.switchThresholdPercent = args.quotaAlertPercent;
          return snap();
        case "set_notify_on_reset":
          state.automation.notifyOnReset = args.enabled;
          return snap();
        case "set_switch_shortcut":
          if (args.shortcut === "Ctrl+Shift+F12") throw L("Bu kısayol başka bir uygulamada kullanılıyor", "This shortcut is already used by another app"); // preview of the error toast
          state.automation.switchShortcut = args.shortcut;
          return snap();
        case "get_journal":
          await sleep(200);
          return journal.slice(0, args.limit);
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
        case "move_account": {
          const i = state.accounts.findIndex((x) => x.accountUuid === args.accountUuid);
          const j = args.direction === "up" ? i - 1 : i + 1;
          if (i >= 0 && j >= 0 && j < state.accounts.length) {
            [state.accounts[i], state.accounts[j]] = [state.accounts[j], state.accounts[i]];
            state.automation.accountOrder = state.accounts.map((x) => x.accountUuid);
          }
          return snap();
        }
        case "remove_account": {
          const a = state.accounts.find((x) => x.accountUuid === args.accountUuid);
          if (!a) throw L("Hesap bulunamadı.", "Account not found.");
          if (a.isActive) throw L("Bu hesapla şu an Claude Desktop'ta giriş yapılmış; kaldırmak için önce başka bir hesaba geç.", "This account is signed in to Claude Desktop right now; switch to another account before removing it.");
          state.accounts = state.accounts.filter((x) => x !== a);
          state.automation.hiddenAccounts.push(a.accountUuid);
          await sleep(300);
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
          return health();
        case "search_chats": {
          await sleep(500);
          const q = String(args.query).toLowerCase();
          return state.sessions
            .slice(0, 4)
            .map((c) => ({
              cliSessionId: c.cliSessionId,
              title: c.title,
              accountUuid: c.accountUuid,
              snippet: `…bunu konuşmuştuk, ${q} konusunda önce örnek bir yapı kurup sonra ayrıntılara geçelim…`,
              matchedRole: c.createdAt % 2 ? "user" : "assistant",
              lastActivityAt: c.lastActivityAt,
            }));
        }
        case "export_chat_markdown": {
          await sleep(300);
          const c = state.sessions.find((x) => x.cliSessionId === args.cliSessionId);
          return `C:\\Users\\ornek\\Downloads\\Claude Sohbetleri\\${c?.title ?? L("Sohbet", "Chat")} (2025-01-15).md`;
        }
        case "reveal_in_explorer":
          console.info("[devMock] reveal", args.path);
          return null;
        default:
          console.warn("[devMock] unhandled command", cmd, args);
          return null;
      }
    },
    { shouldMockEvents: true }
  );
  // Open the preview with ?secim to see the tray's "switch-choice-needed" flow.
  if (location.search.includes("secim")) setTimeout(() => emit("switch-choice-needed", { targetAccountUuid: U2 }), 800);
  // The background check's banner; the real app sends it at most once per version.
  setTimeout(() => emit("update-available", { version: "1.1.0" }), 1200);
}

const scans = new Map<number, Promise<void>>();

// Made-up token counts in real proportions: cache reads ~30x the processed (work) tokens, which are
// ~0.5% input, ~16% output, ~84% cache write. A long chat re-reads more, so ratios differ per row.
// The 30-day view is a scaled-up 7-day one plus one older project.
function spending(days: number): SpendingReport {
  const k = days > 7 ? 3.4 : 1;
  const t = (work: number, reads: number): TokenSums => {
    const w = Math.round(work * k);
    const input = Math.round(w * 0.005), output = Math.round(w * 0.157), cacheWrite = w - input - output;
    const cacheRead = Math.round(w * reads);
    return { input, output, cacheWrite, cacheRead, total: w + cacheRead, work: w };
  };
  const projects = [
    { cwd: "C:\\Users\\demo\\Projects\\web-shop", tokens: t(98_400_000, 32), chats: 9, lastActivityAt: now - 2 * H },
    { cwd: "C:\\Users\\demo\\Projects\\api-server", tokens: t(61_200_000, 22), chats: 5, lastActivityAt: now - 20 * H },
    { cwd: "C:\\Users\\demo\\Documents\\notes", tokens: t(33_700_000, 40), chats: 3, lastActivityAt: now - 50 * H },
    { cwd: "D:\\work\\data-pipeline", tokens: t(20_000_000, 25), chats: 2, lastActivityAt: now - 90 * H },
    ...(days > 7 ? [{ cwd: "D:\\work\\old-prototype", tokens: t(4_100_000, 18), chats: 1, lastActivityAt: now - 400 * H }] : []),
  ];
  const chats = [
    { cliSessionId: "cli-mock-1", title: "Ödeme sayfası yeniden tasarım", cwd: projects[0].cwd, tokens: t(41_300_000, 38), lastActivityAt: now - 2 * H },
    { cliSessionId: "cli-mock-2", title: "Sipariş API'si testleri", cwd: projects[1].cwd, tokens: t(33_900_000, 21), lastActivityAt: now - 20 * H },
    { cliSessionId: "cli-mock-3", title: "Sepet hatası", cwd: projects[0].cwd, tokens: t(27_600_000, 19), lastActivityAt: now - 7 * H },
    { cliSessionId: "cli-unknown", title: L("Adsız sohbet", "Untitled chat"), cwd: projects[0].cwd, tokens: t(18_100_000, 44), lastActivityAt: now - 30 * H },
    { cliSessionId: "cli-mock-4", title: "Toplantı notlarını düzenle", cwd: projects[2].cwd, tokens: t(15_200_000, 40), lastActivityAt: now - 50 * H },
    { cliSessionId: "cli-mock-5", title: "CSV içe aktarma", cwd: projects[3].cwd, tokens: t(9_800_000, 25), lastActivityAt: now - 90 * H },
  ];
  const sum = (f: keyof TokenSums) => projects.reduce((s, p) => s + p.tokens[f], 0);
  const [total, work] = [sum("total"), sum("work")];
  const model = (name: string, share: number, reads: number) => ({ model: name, work: Math.round(work * share), total: Math.round(work * share * (1 + reads)) });
  return {
    days,
    totals: { tokens: { input: sum("input"), output: sum("output"), cacheWrite: sum("cacheWrite"), cacheRead: sum("cacheRead"), total, work }, messages: Math.round(1840 * k), chats: days > 7 ? 20 : 19 },
    projects,
    chats,
    models: [model("claude-opus-4", 0.62, 31), model("claude-sonnet-4", 0.31, 27), model("claude-haiku-4", 0.07, 12)],
    skippedFiles: 0,
  };
}
