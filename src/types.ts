export interface DiscoveredDataRoot {
  path: string;
  sessionsDir: string;
  kind: string;
  hasSessionsDir: boolean;
  hasConfigJson: boolean;
  lastKnownAccountUuid: string | null;
  accountCount: number;
  sessionCount: number;
  isActiveRoot: boolean;
}

export interface EnvironmentReport {
  activeDataRoot: DiscoveredDataRoot | null;
  allDiscoveredRoots: DiscoveredDataRoot[];
  cliConfigHome: string;
  cliGlobalConfig: string;
  cliCredentialsPath: string;
  cliProjectsDir: string;
  cliTranscriptCount: number;
  desktopExePath: string | null;
  desktopRunning: boolean;
  backupStorageDir: string;
}

export interface UsageWindow {
  utilization: number;
  resetsAt: string | null;
}

export interface ExtraUsageInfo {
  isEnabled: boolean;
  usedCredits: number;
  monthlyLimit: number;
  currency: string;
}

export interface UsageMetrics {
  fiveHour: UsageWindow | null;
  sevenDay: UsageWindow | null;
  sevenDaySonnet: UsageWindow | null;
  extraUsage: ExtraUsageInfo | null;
  fetchedAt: string | null;
  source: string;
  rateLimitNotice: string | null;
}

export interface AccountProfile {
  id: string;
  label: string;
  customLabel?: string | null;
  color?: string | null;
  note?: string | null;
  email: string;
  accountUuid: string;
  orgUuid: string;
  planType: string;
  rateLimitTier: string;
  hasDpapiCredentials: boolean;
  isActive: boolean;
  sessionCount: number;
  healthySessionCount: number;
  lastActivityAt: number | null;
  usage: UsageMetrics;
  addedAt: string;
}

export interface AutomationSettings {
  autoSyncEnabled: boolean;
  autoSwitchEnabled: boolean;
  switchThresholdPercent: number;
  notifyOnReset: boolean;
  /** Global shortcut that switches to the next account; "" = off. */
  switchShortcut: string;
  pollIntervalSeconds: number;
  syncSidebarGroups: boolean;
  confirmBeforeDesktopRestart: boolean;
  /** "ask": only approved chats are copied, the rest are asked about at switch time; "all": everything, never ask. */
  transferMode: "ask" | "all";
  /** Background update check (GitHub Releases) and install at startup. */
  autoUpdate: boolean;
  /** Spending alert limits in USD at API list prices; 0 = off. */
  budgetDailyUsd: number;
  budgetWeeklyUsd: number;
  selectedChats: string[];
  selectedGroups: string[];
  selectedFolders: string[];
  /** `chat:<cli>` | `group:<id>` | `folder:<path>` -> account uuids that should have it; missing = all. */
  selectedTargets: Record<string, string[]>;
  /** Accounts removed from the app; they come back when signed into Claude Desktop again. */
  hiddenAccounts: string[];
  /** User-chosen account order (uuids); the dashboard's `accounts` already follow it. */
  accountOrder: string[];
  /** UI language setting; "auto" follows the Windows display language. */
  language: "auto" | "tr" | "en";
}

export interface SidebarGroup {
  id: string;
  name: string;
}

export interface SessionRecord {
  sessionId: string;
  fileName: string;
  filePath: string;
  accountUuid: string;
  accountLabel: string;
  orgUuid: string;
  cliSessionId: string | null;
  cwd: string | null;
  originCwd: string | null;
  title: string;
  createdAt: number;
  lastActivityAt: number;
  completedTurns: number;
  model: string;
  isArchived: boolean;
  isStarred: boolean;
  transcriptUnavailable: boolean;
  transcriptExistsOnDisk: boolean;
  errorNotice: string | null;
  spawnedFrom: string | null;
  groupId: string | null;
}

export interface BackupFileEntry {
  relativePath: string;
  sha256: string;
  sizeBytes: number;
  verified: boolean;
}

export interface BackupManifest {
  id: string;
  createdAt: string;
  reason: string;
  sourceRoot: string;
  backupDir: string;
  totalFiles: number;
  totalBytes: number;
  allVerified: boolean;
  files: BackupFileEntry[];
}

export interface RestorePreviewFile {
  relativePath: string;
  status: "missing" | "different" | "identical";
  title: string;
  lastActivityAt: number | null;
  accountUuid: string;
}

export interface RestorePreview {
  backupId: string;
  files: RestorePreviewFile[];
}

export interface FullDashboardPayload {
  environment: EnvironmentReport;
  automation: AutomationSettings;
  accounts: AccountProfile[];
  sessions: SessionRecord[];
  backups: BackupManifest[];
  lastSyncAt: string | null;
  sidebarGroups: SidebarGroup[];
  formatWarning: string | null;
  /** `automation.language` resolved by the backend. */
  effectiveLanguage: "tr" | "en";
}

export interface HealthItem {
  kind: "chat" | "backup";
  owner: string;
  subject: string;
  problem: string;
}

export interface HealthReport {
  checkedChats: number;
  checkedBackups: number;
  chatProblems: number;
  backupProblems: number;
  items: HealthItem[];
}

export interface PendingChat {
  cliSessionId: string;
  sessionId: string;
  title: string;
  cwd: string | null;
  lastActivityAt: number;
  sourceAccountUuid: string;
}

export interface SwitchPlan {
  ask: boolean;
  pending: PendingChat[];
}

/** Payload of the backend's "switch-suggested" event; `window` is already localized. */
export interface SwitchSuggested {
  fromUuid: string;
  toUuid: string;
  window: string;
  pct: number;
  toPct: number;
}

export interface UpdateInfo {
  available: boolean;
  version: string | null;
  notes: string | null;
  currentVersion: string;
}

export interface UsageSample {
  t: number;
  accountUuid: string;
  fh: number | null;
  sd: number | null;
  src: string;
}

export interface UsageHistory {
  samples: UsageSample[];
  resets: number[];
  forecast: string;
}

export interface SearchHit {
  cliSessionId: string;
  title: string;
  accountUuid: string;
  snippet: string;
  matchedRole: "user" | "assistant";
  lastActivityAt: number;
}

export interface BulkExport {
  folder: string;
  exported: number;
  skipped: { title: string; reason: string }[];
}

export interface JournalEntry {
  timestamp: string;
  operation: string;
  mode: string;
  details: string;
  status: string;
}

export interface TokenSums {
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
  total: number;
  /** input + output + cacheWrite: rankings and shares use this, not `total`. */
  work: number;
}

/** USD at API list prices. */
export interface Cost {
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
  total: number;
}

export type SpendingRange = "today" | "7" | "30" | "90" | "all";
/** Work tokens and USD of one bucket. */
export interface Amount {
  work: number;
  cost: number;
}

export interface SpendingReport {
  range: SpendingRange;
  /** Local midnight of the window's first day (ms). */
  start: number;
  totals: {
    tokens: TokenSums;
    cost: Cost;
    messages: number;
    chats: number;
    /** Tokens of models with no list price (left out of `cost`). */
    unpriced: TokenSums;
    subagentTokens: TokenSums;
    subagentCost: number;
  };
  /** The same span right before; null for all time. */
  previous: { tokens: TokenSums; cost: number } | null;
  projects: { cwd: string; tokens: TokenSums; cost: number; messages: number; chats: number; lastActivityAt: number }[];
  chats: { cliSessionId: string; title: string; cwd: string; tokens: TokenSums; cost: number; messages: number; lastActivityAt: number }[];
  /** `cost` null: no list price for this model. */
  models: { model: string; family: string; tokens: TokenSums; cost: number | null; messages: number; lastActivityAt: number }[];
  /** One per local day, oldest first; families: fable | mythos | opus | sonnet | haiku | other. */
  days: { start: number; families: Record<string, Amount> }[];
  /** By local hour of day, 0-23. */
  hours: Amount[];
  prices: { model: string; input: number; output: number; cacheWrite: number; cacheRead: number }[];
  pricesAsOf: string;
  skippedFiles: number;
}

/** `get_spending_budget`: cost of today and of the last 7 local days (the "today" and "7" report totals). */
export interface BudgetSpent {
  today: number;
  week: number;
}

export interface DiskSize {
  bytes: number;
  files: number;
  /** Newest file (ms); 0 with no files. */
  lastModified: number;
}

export interface DiskUsage {
  transcripts: DiskSize;
  /** Largest first; `path` is the project's folder when known, else the transcript folder name. */
  projects: { folder: string; path: string; size: DiskSize }[];
  backups: { total: DiskSize; full: { bytes: number; count: number }; liveRefresh: { bytes: number; count: number } };
  pointers: DiskSize;
  app: DiskSize;
}

export type DiskFolderKind = "transcripts" | "backups" | "pointers" | "app";
