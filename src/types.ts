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
  pollIntervalSeconds: number;
  syncSidebarGroups: boolean;
  confirmBeforeDesktopRestart: boolean;
  /** "ask": only approved chats are copied, the rest are asked about at switch time; "all": everything, never ask. */
  transferMode: "ask" | "all";
  selectedChats: string[];
  selectedGroups: string[];
  selectedFolders: string[];
  /** `chat:<cli>` | `group:<id>` | `folder:<path>` -> account uuids that should have it; missing = all. */
  selectedTargets: Record<string, string[]>;
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

export interface FullDashboardPayload {
  environment: EnvironmentReport;
  automation: AutomationSettings;
  accounts: AccountProfile[];
  sessions: SessionRecord[];
  backups: BackupManifest[];
  lastSyncAt: string | null;
  sidebarGroups: SidebarGroup[];
  formatWarning: string | null;
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
