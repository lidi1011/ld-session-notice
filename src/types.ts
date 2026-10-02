export type NoticeLevel = "info" | "success" | "warning" | "error";

export type AppLocale = "en" | "zh-CN";

export type TrafficWidgetManualState = "ready" | "running" | "waiting" | "failed" | "complete";

export type NoticeEventType =
  | "SESSION_IDLE"
  | "SESSION_END"
  | "TASK_START"
  | "TASK_FINISH"
  | "TASK_FAIL"
  | "USER_CONFIRM"
  | "WARNING"
  | "ERROR";

export interface NoticeEvent {
  id: string;
  version: 1;
  provider: "codex" | "claude_code" | "webhook";
  eventType: NoticeEventType;
  sessionId?: string;
  runId?: string;
  dedupeKey?: string;
  title: string;
  content: string;
  level: NoticeLevel;
  project?: string;
  cwd?: string;
  command?: string;
  exitCode?: number;
  durationMs?: number;
  timestamp: string;
  receivedAt: string;
  rawPayload?: unknown;
}

export interface DashboardSummary {
  serviceStatus: string;
  todayTotal: number;
  todaySuccess: number;
  todayFailure: number;
  todayConfirmations: number;
  recentSummary?: string;
}

export interface EventFilter {
  search?: string;
  level?: NoticeLevel | "";
  project?: string;
}

export interface Pagination {
  page: number;
  pageSize: number;
}

export interface ChannelConfig {
  webhookMasked?: string;
  hasWebhook: boolean;
  hasSignSecret: boolean;
  enabled: boolean;
  lastStatus?: string;
}

export interface HookStatus {
  installed: boolean;
  configPath: string;
  managedBlockHash?: string;
  backupPath?: string;
  message: string;
}

export interface HookPreview {
  configPath: string;
  willCreateConfig: boolean;
  preview: string;
}

export interface PendingApproval {
  id: string;
  command: string;
  project?: string;
  riskLevel: string;
  rule: string;
  status: "pending" | "approved" | "rejected" | "timed_out";
  createdAt: string;
  expiresAt: string;
}

export interface ProviderTrafficStatus {
  activeSessions: number;
  pendingApprovals: number;
  failures: number;
  complete: boolean;
  activityStartedAt?: string;
}

export interface TrafficWidgetStatus {
  enabled: boolean;
  alwaysOnTop: boolean;
  color: "red" | "yellow" | "green" | "running";
  label: string;
  detail: string;
  activeSessions: number;
  codexActiveSessions: number;
  claudeActiveSessions: number;
  codexStatus: ProviderTrafficStatus;
  claudeStatus: ProviderTrafficStatus;
  pendingApprovals: number;
  todayFailures: number;
  latestEventTitle?: string;
  codexUsage?: CodexUsageStatus;
  claudeWeeklyRemaining?: number;
  manualOverride?: TrafficWidgetManualState;
}

export interface CodexUsageStatus {
  limitId: string;
  limitName?: string;
  primary?: CodexUsageWindow;
  secondary?: CodexUsageWindow;
  planType?: string;
  rateLimitReachedType?: string;
  updatedAt: string;
}

export interface CodexUsageWindow {
  usedPercent: number;
  remainingPercent: number;
  windowMinutes: number;
  resetsAt?: string;
}

export interface PetConfig {
  enabled: boolean;
  baseUrl?: string;
  lastStatus?: string;
}

export interface MochiVoiceConfig {
  enabled: boolean;
  serialPort: string;
  asrUrl: string;
  lastStatus?: string;
}

export interface ClaudeUsageStatus {
  remainingPercent?: number;
  updatedAt?: string;
  message: string;
}
