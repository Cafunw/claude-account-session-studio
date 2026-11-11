// Dev preview only (plain `vite` in a browser, no Tauri backend). Every value here is made up.
// main.tsx imports this behind `import.meta.env.DEV`, so it never reaches the production bundle.
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { AccountProfile, EnvironmentReport, FullDashboardPayload, HealthReport, PendingChat, SessionRecord, SpendingRange, SpendingReport, TokenSums, UsageHistory } from "./types";

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
        case "get_spending": {
          // First open of each range acts like a cold scan with progress events; later ones are warm.
          // A second request joins the running scan, as the backend does.
          const range: SpendingRange = args.range;
          if (!scans.has(range)) {
            const total = { today: 212, "7": 1406, "30": 2686, "90": 3920, all: 5551 }[range];
            const slow = location.search.includes("yavas");
            scans.set(range, (async () => {
              for (let i = 0; i <= 20; i++) {
                await emit("spending-progress", { done: Math.round((total * i) / 20), total });
                await sleep(slow ? 1500 : 120);
              }
            })());
          } else await sleep(400);
          await scans.get(range);
          return location.search.includes("bos") ? emptySpending(range) : spending(range);
        }
        case "export_spending_csv":
          await sleep(300);
          return `C:\\Users\\demo\\Downloads\\claude-harcama-${{ today: "bugun", all: "tum-zamanlar" }[args.range as string] ?? `${args.range}-gun`}-${new Date().toISOString().slice(0, 10)}.csv`;
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

  // Screenshot flags: ?tema=koyu|acik, ?dil=tr|en, and ?harcama opens the Spending page; with it
  // ?usd (API $), ?tum (all time), ?ipucu (a chart tooltip), ?fiyat (price table), ?proje (project filter).
  const q = new URLSearchParams(location.search);
  const theme = q.get("tema");
  if (theme) {
    try {
      localStorage.setItem("theme", theme === "koyu" ? "dark" : "light");
    } catch {}
  }
  const lang = q.get("dil");
  if (lang === "tr" || lang === "en") state.automation.language = lang;
  if (q.has("harcama")) void spendingTour(q);
}

async function spendingTour(q: URLSearchParams) {
  const until = async (find: () => Element | null | undefined) => {
    for (let i = 0; i < 100; i++) {
      const el = find();
      if (el) return el as HTMLElement;
      await sleep(100);
    }
    return null;
  };
  const radio = (group: number, index: number) => () => document.querySelectorAll("[role=radiogroup]")[group]?.querySelectorAll("[role=radio]")[index];
  (await until(() => [...document.querySelectorAll("nav button")].find((b) => ["Harcama", "Spending"].includes(b.textContent?.trim() ?? ""))))?.click();
  if (q.has("tum")) (await until(radio(0, 4)))?.click();
  if (q.has("usd")) (await until(radio(1, 1)))?.click();
  if (!q.has("ipucu") && !q.has("fiyat") && !q.has("proje")) return;
  await until(() => document.querySelector("dl"));
  await sleep(300);
  if (q.has("fiyat")) (await until(() => document.querySelector("details")))?.setAttribute("open", "");
  if (q.has("proje")) (await until(() => document.querySelector("td button")))?.click();
  if (q.has("ipucu")) {
    const bars = document.querySelectorAll("svg rect[tabindex='0']");
    (bars[Math.max(0, bars.length - 3)] as SVGElement | undefined)?.focus();
  }
}

const scans = new Map<SpendingRange, Promise<void>>();

// Made-up usage in real proportions: per family, work tokens are ~0.5% input, ~16% output and ~84%
// cache write, with ~30x as many cache reads; USD at the backend's price table.
const MOCK_PRICES = [
  ["claude-fable-5-2", 8, 40, 10, 0.2],
  ["claude-mythos-5-2", 8, 40, 10, 0.2],
  ["claude-fable-5-1", 10, 50, 12.5, 0.25],
  ["claude-mythos-5-1", 10, 50, 12.5, 0.25],
  ["claude-fable-5", 10, 50, 12.5, 1],
  ["claude-mythos-5", 10, 50, 12.5, 1],
  ["claude-opus-5-5", 4, 20, 5, 0.2],
  ["claude-opus-5", 5, 25, 6.25, 0.5],
  ["claude-opus-4-8", 5, 25, 6.25, 0.5],
  ["claude-opus-4-7", 5, 25, 6.25, 0.5],
  ["claude-opus-4-6", 5, 25, 6.25, 0.5],
  ["claude-sonnet-5-5", 2, 10, 2.5, 0.1],
  ["claude-sonnet-5", 2, 10, 2.5, 0.2],
  ["claude-sonnet-4-6", 3, 15, 3.75, 0.3],
  ["claude-haiku-5-5", 0.1, 0.5, 0.125, 0.01],
  ["claude-haiku-4-5", 1, 5, 1.25, 0.1],
] as const;
// [family, model id, share of work, cache reads per work token]
const MOCK_MODELS = [
  ["opus", "claude-opus-4-8", 0.6, 31],
  ["sonnet", "claude-sonnet-5-5", 0.27, 27],
  ["haiku", "claude-haiku-5-5", 0.06, 12],
  ["fable", "claude-fable-5-2", 0.05, 22],
  ["other", "claude-sonnet-4-5-20250929", 0.02, 25],
] as const;
const zeroTokens = (): TokenSums => ({ input: 0, output: 0, cacheWrite: 0, cacheRead: 0, total: 0, work: 0 });
const zeroCost = () => ({ input: 0, output: 0, cacheWrite: 0, cacheRead: 0, total: 0 });
const split = (work: number, reads: number): TokenSums => {
  const input = Math.round(work * 0.005), output = Math.round(work * 0.157), cacheWrite = work - input - output, cacheRead = Math.round(work * reads);
  return { input, output, cacheWrite, cacheRead, total: work + cacheRead, work };
};
const priceOf = (model: string) => MOCK_PRICES.find((p) => p[0] === model);
const costOf = (model: string, tk: TokenSums) => {
  const p = priceOf(model);
  if (!p) return null;
  const c = { input: (tk.input * p[1]) / 1e6, output: (tk.output * p[2]) / 1e6, cacheWrite: (tk.cacheWrite * p[3]) / 1e6, cacheRead: (tk.cacheRead * p[4]) / 1e6, total: 0 };
  c.total = c.input + c.output + c.cacheWrite + c.cacheRead;
  return c;
};
const addTokens = (a: TokenSums, b: TokenSums) => (Object.keys(a) as (keyof TokenSums)[]).forEach((k) => (a[k] += b[k]));
const scaleTokens = (tk: TokenSums, f: number): TokenSums => {
  const s = split(Math.round(tk.work * f), 0);
  return { ...s, cacheRead: Math.round(tk.cacheRead * f), total: s.work + Math.round(tk.cacheRead * f) };
};

function emptySpending(range: SpendingRange): SpendingReport {
  return { ...spending(range), totals: { tokens: zeroTokens(), cost: zeroCost(), messages: 0, chats: 0, unpriced: zeroTokens(), subagentTokens: zeroTokens(), subagentCost: 0 }, previous: null, projects: [], chats: [], models: [], days: [], hours: [] };
}

function spending(range: SpendingRange): SpendingReport {
  const n = { today: 1, "7": 7, "30": 30, "90": 90, all: 210 }[range];
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
  const midnight = new Date();
  midnight.setHours(0, 0, 0, 0);
  // Fixed daily pattern counted back from today, so every range agrees with the others.
  const daily = Array.from({ length: 210 }, (_, back) => {
    const d = new Date(midnight);
    d.setDate(d.getDate() - back);
    const weekend = d.getDay() === 0 || d.getDay() === 6;
    const quiet = rnd() < 0.12 && back > 0;
    return { start: d.getTime(), work: quiet ? 0 : Math.round((weekend ? 9e6 : 31e6) * (0.55 + rnd()) * (back === 0 ? 0.6 : 1)) };
  });
  const days = daily.slice(0, n).reverse().map((d, i) => {
    const families: Record<string, { work: number; cost: number }> = {};
    for (const [fam, model, share, reads] of MOCK_MODELS) {
      if (fam === "fable" && n - i > 24) continue; // Fable arrived three weeks ago
      const work = Math.round(d.work * share);
      if (work > 0) families[fam] = { work, cost: costOf(model, split(work, reads))?.total ?? 0 };
    }
    return { start: d.start, families };
  });

  const models = MOCK_MODELS.map(([family, model, , reads]) => {
    const work = days.reduce((s, d) => s + (d.families[family]?.work ?? 0), 0);
    const tokens = split(work, reads);
    return { model, family, tokens, cost: costOf(model, tokens)?.total ?? null, messages: Math.round(work / 9000), lastActivityAt: now - (family === "other" ? 26 * 24 * H : 2 * H) };
  }).filter((m) => m.tokens.work > 0);
  const tokens = zeroTokens(), cost = zeroCost(), unpriced = zeroTokens();
  for (const m of models) {
    addTokens(tokens, m.tokens);
    const c = costOf(m.model, m.tokens);
    if (c) (Object.keys(cost) as (keyof typeof cost)[]).forEach((k) => (cost[k] += c[k]));
    else addTokens(unpriced, m.tokens);
  }

  const projectDefs: [string, number, number][] = [
    ["C:\\Users\\demo\\Projects\\web-shop", 0.41, 9],
    ["C:\\Users\\demo\\Projects\\api-server", 0.26, 6],
    ["C:\\Users\\demo\\Documents\\notes", 0.13, 3],
    ["D:\\work\\data-pipeline", 0.09, 2],
    ["C:\\Users\\demo\\Projects\\mobile-app", 0.06, 2],
    ["D:\\work\\old-prototype", 0.05, 1],
  ];
  const projects = projectDefs.map(([cwd, share, chats], i) => ({
    cwd,
    tokens: scaleTokens(tokens, share),
    cost: cost.total * share * (1 + (i % 2 ? -0.08 : 0.06)),
    messages: Math.round((tokens.work * share) / 9000),
    chats,
    lastActivityAt: now - (2 + i * 19) * H,
  }));
  const titles = [
    "Ödeme sayfası yeniden tasarım", "Sipariş API'si testleri", "Sepet hatası", L("Adsız sohbet", "Untitled chat"), "Toplantı notlarını düzenle", "CSV içe aktarma",
    "Ürün arama hızlandırma", "Kimlik doğrulama akışı", "Veritabanı göçü", "Bildirim servisi", "Raporlama ekranı", "Görsel yükleme hatası",
    "Önbellek katmanı", "Fatura PDF çıktısı", "Kod incelemesi notları", "Mobil giriş ekranı", "Günlük özet betiği", "Hata izleme kurulumu",
    "Stok senkronizasyonu", "Fiyat kuralları", "Dokümantasyon güncellemesi", "Eski prototip temizliği",
  ];
  const chats = titles.map((title, i) => {
    const share = 0.2 / (1 + i * 0.45);
    const p = projectDefs[i % projectDefs.length];
    return {
      cliSessionId: `cli-mock-${i + 1}`,
      title,
      cwd: p[0],
      tokens: scaleTokens(tokens, share),
      cost: cost.total * share * (i % 3 === 0 ? 1.1 : 0.93),
      messages: Math.round((tokens.work * share) / 9000),
      lastActivityAt: now - (2 + i * 7) * H,
    };
  });
  const hourShape = [0.3, 0.1, 0, 0, 0, 0, 0, 0.1, 0.4, 0.8, 1, 0.9, 0.5, 0.6, 0.9, 1, 0.95, 0.7, 0.4, 0.5, 0.75, 0.85, 0.7, 0.5];
  const shapeSum = hourShape.reduce((a, b) => a + b, 0);
  const hours = hourShape.map((v) => ({ work: Math.round((tokens.work * v) / shapeSum), cost: (cost.total * v) / shapeSum }));
  const prevFactor = range === "today" ? 1.3 : range === "30" ? 0.92 : 0.88;

  return {
    range,
    start: days[0].start,
    totals: {
      tokens,
      cost,
      messages: Math.round(tokens.work / 9000),
      chats: Math.min(titles.length, Math.max(3, Math.round(n * 2.6))),
      unpriced,
      subagentTokens: scaleTokens(tokens, 0.18),
      subagentCost: cost.total * 0.16,
    },
    previous: range === "all" ? null : { tokens: scaleTokens(tokens, prevFactor), cost: cost.total * prevFactor },
    projects,
    chats,
    models,
    days,
    hours,
    prices: MOCK_PRICES.map(([model, input, output, cacheWrite, cacheRead]) => ({ model, input, output, cacheWrite, cacheRead })),
    pricesAsOf: "2026-11-03",
    skippedFiles: 0,
  };
}
