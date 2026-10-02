<script setup lang="ts">
import { computed, h, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { DataTableColumns, MenuOption } from "naive-ui";
import { darkTheme, dateEnUS, dateZhCN, enUS, zhCN } from "naive-ui";
import { useApprovalStore } from "./stores/approvals";
import { useChannelStore } from "./stores/channel";
import { useDashboardStore } from "./stores/dashboard";
import { useEventsStore } from "./stores/events";
import { useHookStore } from "./stores/hooks";
import { useTrafficStore } from "./stores/traffic";
import { api } from "./api";
import type {
  AppLocale,
  HookStatus,
  HookPreview,
  ClaudeUsageStatus,
  ProviderTrafficStatus,
  NoticeEvent,
  PendingApproval,
  PetConfig,
  MochiVoiceConfig,
  TrafficWidgetManualState,
} from "./types";

const active = ref("dashboard");
const locale = ref<AppLocale>("en");
const dashboard = useDashboardStore();
const events = useEventsStore();
const channels = useChannelStore();
const hooks = useHookStore();
const claudeHooks = ref<HookStatus | null>(null);
const claudeHookPreview = ref<HookPreview | null>(null);
const claudeUsage = ref<ClaudeUsageStatus | null>(null);
const claudeBusy = ref(false);
const claudeError = ref("");
async function loadClaudeProvider() {
  try {
    [claudeHooks.value, claudeUsage.value] = await Promise.all([api.claudeHookStatus(), api.claudeUsageStatus()]);
  } catch (error) { claudeError.value = String(error); }
}
async function claudeAction(action: "preview" | "install" | "uninstall" | "login" | "refresh") {
  claudeBusy.value = true;
  claudeError.value = "";
  try {
    if (action === "preview") claudeHookPreview.value = await api.previewClaudeHooks();
    if (action === "install") claudeHooks.value = await api.installClaudeHooks();
    if (action === "uninstall") claudeHooks.value = await api.uninstallClaudeHooks();
    if (action === "login") await api.openClaudeAccount();
    if (action === "refresh") claudeUsage.value = await api.refreshClaudeUsage();
    await loadClaudeProvider();
  } catch (error) { claudeError.value = String(error); }
  finally { claudeBusy.value = false; }
}
const approvals = useApprovalStore();
const traffic = useTrafficStore();
const isWidget = new URLSearchParams(window.location.search).get("widget") === "traffic";
let refreshTimer: number | undefined;
let mochiVoiceStatusTimer: number | undefined;
const autostartEnabled = ref(false);
const autostartLoading = ref(false);
const refreshAllLoading = ref(false);
const runtimeRefreshLoading = ref(false);
const refreshFeedback = ref("");
const petConfig = ref<PetConfig | null>(null);
const petBaseUrl = ref("");
const petEnabled = ref(false);
const petLoading = ref(false);
const petTesting = ref(false);
const petFeedback = ref("");
const mochiVoiceConfig = ref<MochiVoiceConfig | null>(null);
const mochiVoiceEnabled = ref(false);
const mochiVoiceSerialPort = ref("/dev/cu.usbmodem1301");
const mochiVoiceAsrUrl = ref("ws://110.42.235.130:10095");
const mochiVoiceLoading = ref(false);
const mochiVoiceFeedback = ref("");
const manualWidgetFeedback = ref("");
type ManualWidgetSelection = "off" | TrafficWidgetManualState;

const copy = {
  en: {
    claudeQuotaRefresh: "Refresh quota",
    claudeSignIn: "Open Claude in browser",
    claudeQuotaRemaining: "Weekly remaining",
    claudeQuotaDescription: "Uses the default browser’s current Claude login. Refreshes independently every 5 minutes.",
    claudeQuotaTitle: "Claude weekly quota",
    claudeHookDescription: "Supports Claude Code CLI and local Code sessions in Claude Desktop.",
    claudeHookManager: "Claude Code Hook Manager",
    dashboard: "Dashboard",
    events: "Events",
    channels: "Channels",
    providers: "Providers",
    approvals: "Approvals",
    settings: "Settings",
    brandSubtitle: "Developer Notification Center",
    service: "Service",
    today: "Today",
    success: "Success",
    failure: "Failure",
    trafficWidget: "Traffic Widget",
    recentSummary: "Recent summary",
    noSummary: "No aggregated summary yet.",
    searchPlaceholder: "Search title, content, command",
    level: "Level",
    project: "Project",
    search: "Search",
    clear: "Clear",
    time: "Time",
    type: "Type",
    title: "Title",
    content: "Content",
    created: "Created",
    risk: "Risk",
    rule: "Rule",
    command: "Command",
    action: "Action",
    approve: "Approve",
    reject: "Reject",
    feishu: "Feishu",
    feishuNotifications: "Feishu notifications",
    feishuEnabled: "Only approval reminders and successful completion are sent to Feishu",
    feishuDisabled: "Notifications are off; saved config is kept",
    webhookSaved: "Webhook saved",
    webhookMissing: "Webhook not configured",
    webhookPrompt: "Enter Webhook URL, then Save",
    webhookUrl: "Webhook URL",
    webhookOverwrite: "Saved; enter a new URL to replace it",
    webhookPaste: "Paste Feishu bot Webhook URL",
    signSecret: "Sign Secret",
    signSecretOverwrite: "Saved; enter a new sign secret to replace it",
    signSecretOptional: "Optional: Feishu sign secret",
    save: "Save",
    sendTest: "Send Test",
    notConfigured: "Not configured",
    configured: "Configured",
    lastStatus: "Last status",
    hookManager: "Codex Hook Manager",
    config: "Config",
    status: "Status",
    preview: "Preview",
    install: "Install",
    uninstall: "Uninstall",
    runtime: "Runtime",
    trafficDescription: "A floating status light for Codex sessions.",
    manualWidgetOverride: "Photo status override",
    manualWidgetOverrideDescription: "Temporarily force the floating widget status without changing event history.",
    manualWidgetSaved: "Photo status updated",
    manualStateOff: "Off",
    manualStateReady: "Ready",
    manualStateRunning: "Running",
    manualStateWaiting: "Waiting",
    manualStateFailed: "Failed",
    manualStateComplete: "Complete",
    petIntegration: "Mochi desktop pet",
    petDescription: "Sync the traffic widget status to your ESP32 Mochi expression screen.",
    petAddress: "Mochi URL",
    petAddressPlaceholder: "192.168.1.23 or http://192.168.1.23",
    petEnabled: "Enable pet sync",
    testPet: "Test pet",
    petSaved: "Pet sync saved",
    petTestSent: "Pet test sent",
    mochiVoiceInput: "Mochi voice input",
    mochiVoiceDescription: "Hold the hardware button to record, then insert the ASR transcript into the current focused input.",
    mochiVoiceEnabled: "Enable voice input",
    mochiVoiceSerialPort: "Serial port",
    mochiVoiceSerialPlaceholder: "/dev/cu.usbmodem1301",
    mochiVoiceAsrUrl: "ASR WebSocket URL",
    mochiVoiceAsrPlaceholder: "ws://110.42.235.130:10095",
    mochiVoiceSaved: "Voice input saved",
    localServer: "Local server",
    retention: "Database retention: 30 days",
    criticalTimeout: "Critical command timeout: 30 seconds, fail-closed",
    refreshAll: "Refresh all",
    refreshing: "Refreshing...",
    refreshDone: "Refreshed",
    refreshFailed: "Refresh failed",
    refreshStatus: "Refresh status",
    statusRefreshDone: "Status refreshed",
    statusRefreshFailed: "Status refresh failed",
    language: "Language",
    languageDescription: "Switch the app interface language.",
    autostart: "Launch at login",
    autostartDescription: "Start Notice automatically after you sign in to macOS.",
    switchToChinese: "Switch to Simplified Chinese",
    switchToEnglish: "Switch to English",
    currentLanguage: "Current language",
    simplifiedChinese: "Simplified Chinese",
    english: "English",
    widgetTitleSuffix: "Right-click for options.",
    codexUsage: "Codex usage",
    ready: "Ready",
    watching: "Notice is watching Codex",
    running: "Running",
    complete: "Complete",
    waiting: "Waiting",
    needsAttention: "Needs attention",
    runningDetail: (count: number) => `${count} Codex session(s) running`,
    waitingDetail: (count: number) => `${count} approval(s) pending`,
    failureDetail: (count: number) => `${count} failed Codex event(s) today`,
    completeDetail: "All tracked Codex tasks are complete",
    readyDetail: "Notice is watching for Codex activity",
  },
  "zh-CN": {
    claudeQuotaRefresh: "刷新额度",
    claudeSignIn: "在浏览器打开 Claude",
    claudeQuotaRemaining: "7 天剩余",
    claudeQuotaDescription: "使用默认浏览器当前个人资料的 Claude 登录态，独立读取额度，每 5 分钟刷新。",
    claudeQuotaTitle: "Claude 7 天额度",
    claudeHookDescription: "支持 Claude Code CLI 和 Claude Desktop 本地 Code 会话。",
    claudeHookManager: "Claude Code Hook 管理",
    dashboard: "仪表盘",
    events: "事件",
    channels: "通知渠道",
    providers: "接入源",
    approvals: "审批",
    settings: "设置",
    brandSubtitle: "开发者通知中心",
    service: "服务",
    today: "今日事件",
    success: "成功",
    failure: "失败",
    trafficWidget: "红绿灯小组件",
    recentSummary: "最近摘要",
    noSummary: "还没有聚合摘要。",
    searchPlaceholder: "搜索标题、内容、命令",
    level: "等级",
    project: "项目",
    search: "搜索",
    clear: "清空",
    time: "时间",
    type: "类型",
    title: "标题",
    content: "内容",
    created: "创建时间",
    risk: "风险",
    rule: "规则",
    command: "命令",
    action: "操作",
    approve: "批准",
    reject: "拒绝",
    feishu: "飞书",
    feishuNotifications: "飞书通知",
    feishuEnabled: "开启后仅审批提醒和成功结束会发送到飞书",
    feishuDisabled: "关闭后不会发送任务通知，配置仍会保留",
    webhookSaved: "Webhook 已保存",
    webhookMissing: "Webhook 未配置",
    webhookPrompt: "输入 Webhook URL 后点击保存",
    webhookUrl: "Webhook URL",
    webhookOverwrite: "已保存，输入新 URL 可覆盖",
    webhookPaste: "粘贴飞书机器人 Webhook URL",
    signSecret: "签名密钥",
    signSecretOverwrite: "已保存，输入新签名密钥可覆盖",
    signSecretOptional: "可选：飞书签名密钥",
    save: "保存",
    sendTest: "发送测试",
    notConfigured: "未配置",
    configured: "已配置",
    lastStatus: "最近状态",
    hookManager: "Codex Hook 管理",
    config: "配置",
    status: "状态",
    preview: "预览",
    install: "安装",
    uninstall: "卸载",
    runtime: "运行时",
    trafficDescription: "独立悬浮在桌面的 Codex 会话状态灯。",
    manualWidgetOverride: "拍照状态覆盖",
    manualWidgetOverrideDescription: "临时强制小组件状态，不会写入事件历史。",
    manualWidgetSaved: "拍照状态已更新",
    manualStateOff: "关闭覆盖",
    manualStateReady: "就绪",
    manualStateRunning: "运行中",
    manualStateWaiting: "等待确认",
    manualStateFailed: "失败",
    manualStateComplete: "已完成",
    petIntegration: "Mochi 桌宠",
    petDescription: "把红绿灯状态同步到 ESP32 Mochi 表情屏。",
    petAddress: "Mochi 地址",
    petAddressPlaceholder: "192.168.1.23 或 http://192.168.1.23",
    petEnabled: "开启桌宠同步",
    testPet: "测试桌宠",
    petSaved: "桌宠同步已保存",
    petTestSent: "桌宠测试已发送",
    mochiVoiceInput: "Mochi 语音输入",
    mochiVoiceDescription: "按住硬件按钮录音，松开后转写，并把文字输入到当前键盘焦点所在位置。",
    mochiVoiceEnabled: "开启语音输入",
    mochiVoiceSerialPort: "串口",
    mochiVoiceSerialPlaceholder: "/dev/cu.usbmodem1301",
    mochiVoiceAsrUrl: "ASR WebSocket 地址",
    mochiVoiceAsrPlaceholder: "ws://110.42.235.130:10095",
    mochiVoiceSaved: "语音输入已保存",
    localServer: "本地服务",
    retention: "数据库保留：30 天",
    criticalTimeout: "高风险命令超时：30 秒，默认拒绝",
    refreshAll: "刷新全部",
    refreshing: "刷新中...",
    refreshDone: "已刷新",
    refreshFailed: "刷新失败",
    refreshStatus: "刷新状态",
    statusRefreshDone: "状态已刷新",
    statusRefreshFailed: "状态刷新失败",
    language: "语言",
    languageDescription: "切换应用界面语言。",
    autostart: "开机自启动",
    autostartDescription: "登录 macOS 后自动启动 Notice。",
    switchToChinese: "一键切换为简体中文",
    switchToEnglish: "切换为英文",
    currentLanguage: "当前语言",
    simplifiedChinese: "简体中文",
    english: "英文",
    widgetTitleSuffix: "右键可设置选项。",
    codexUsage: "Codex 用量",
    ready: "就绪",
    watching: "Notice 正在监听 Codex",
    running: "运行中",
    complete: "已完成",
    waiting: "等待确认",
    needsAttention: "需要处理",
    runningDetail: (count: number) => `${count} 个 Codex 会话运行中`,
    waitingDetail: (count: number) => `${count} 个审批待处理`,
    failureDetail: (count: number) => `今日 ${count} 个 Codex 事件失败`,
    completeDetail: "所有已跟踪的 Codex 任务均已成功完成",
    readyDetail: "Notice 正在等待 Codex 活动",
  },
} as const;

type CopyKey = keyof typeof copy.en;
type TextKey = {
  [K in CopyKey]: (typeof copy.en)[K] extends string ? K : never;
}[CopyKey];
type FormatKey = Exclude<CopyKey, TextKey>;

function t(key: TextKey): string {
  return copy[locale.value][key] as string;
}

function tf(key: FormatKey, count: number): string {
  return (copy[locale.value][key] as (count: number) => string)(count);
}

const naiveLocale = computed(() => (locale.value === "zh-CN" ? zhCN : enUS));
const naiveDateLocale = computed(() => (locale.value === "zh-CN" ? dateZhCN : dateEnUS));
const menuOptions = computed<MenuOption[]>(() => [
  { label: t("dashboard"), key: "dashboard" },
  { label: t("events"), key: "events" },
  { label: t("channels"), key: "channels" },
  { label: t("providers"), key: "providers" },
  { label: t("approvals"), key: "approvals" },
  { label: t("settings"), key: "settings" },
]);

const eventColumns = computed<DataTableColumns<NoticeEvent>>(() => [
  { title: t("time"), key: "receivedAt", width: 170 },
  { title: t("project"), key: "project", width: 140 },
  {
    title: t("level"),
    key: "level",
    width: 105,
    render: (row) => h("span", { class: `level level-${row.level}` }, row.level),
  },
  { title: t("type"), key: "eventType", width: 150 },
  { title: t("title"), key: "title" },
  { title: t("content"), key: "content" },
]);

const approvalColumns = computed<DataTableColumns<PendingApproval>>(() => [
  { title: t("created"), key: "createdAt", width: 170 },
  { title: t("project"), key: "project", width: 130 },
  { title: t("risk"), key: "riskLevel", width: 90 },
  { title: t("rule"), key: "rule", width: 150 },
  { title: t("command"), key: "command" },
  {
    title: t("action"),
    key: "actions",
    width: 190,
    render: (row) =>
      h("div", { class: "row-actions" }, [
        h(
          "button",
          { class: "plain-button approve", onClick: () => approvals.resolve(row.id, "approved") },
          t("approve"),
        ),
        h(
          "button",
          { class: "plain-button reject", onClick: () => approvals.resolve(row.id, "rejected") },
          t("reject"),
        ),
      ]),
  },
]);

const currentTitle = computed(() => {
  const item = menuOptions.value.find((option) => option.key === active.value);
  return String(item?.label ?? "Notice");
});
const trafficClass = computed(() => `traffic-widget traffic-${traffic.status?.color ?? "green"}`);
const trafficLabel = computed(() => {
  const color = traffic.status?.color;
  if (color === "running") return t("running");
  if (color === "red") return t("needsAttention");
  if (traffic.status?.pendingApprovals) return t("waiting");
  if (color === "green" && traffic.status?.latestEventTitle) return t("complete");
  return t("ready");
});
function providerDetail(status?: ProviderTrafficStatus) {
  let text: string;
  if (!status) text = t("watching");
  else if (status.failures > 0) text = tf("failureDetail", status.failures);
  else if (status.pendingApprovals > 0) text = tf("waitingDetail", status.pendingApprovals);
  else if (status.activeSessions > 0) text = tf("runningDetail", status.activeSessions);
  else if (status.complete) text = t("completeDetail");
  else text = t("readyDetail");
  // The source is already named by the row prefix.
  return text.replace(/\s*Codex\s*/g, locale.value === "zh-CN" ? "" : " ");
}
const codexWeeklyRemaining = computed(() => {
  const usage = traffic.status?.codexUsage;
  const weekly = [usage?.primary, usage?.secondary].find(window => window?.windowMinutes === 7 * 24 * 60);
  return weekly?.remainingPercent;
});
function quotaText(value?: number) {
  return typeof value === "number" && Number.isFinite(value)
    ? `${Math.round(Math.min(100, Math.max(0, value)))}%` : "";
}
const providerCandidates = computed(() => [
  { key: "codex", name: "Codex", status: traffic.status?.codexStatus, quota: quotaText(codexWeeklyRemaining.value) },
  { key: "claude", name: "Claude", status: traffic.status?.claudeStatus, quota: quotaText(traffic.status?.claudeWeeklyRemaining) },
].filter(row => row.status && (row.status.activeSessions > 0 || row.status.pendingApprovals > 0 || row.status.failures > 0)));
const providerOrder = ref<string[]>([]);
watch(providerCandidates, rows => {
  const visible = new Set(rows.map(row => row.key));
  const retained = providerOrder.value.filter(key => visible.has(key));
  const added = rows.filter(row => !retained.includes(row.key)).sort((a, b) =>
    (Date.parse(a.status?.activityStartedAt ?? "") || 0) - (Date.parse(b.status?.activityStartedAt ?? "") || 0));
  providerOrder.value = [...retained, ...added.map(row => row.key)];
}, { immediate: true });
const providerRows = computed(() => providerOrder.value.flatMap(key => {
  const row = providerCandidates.value.find(row => row.key === key);
  return row ? [{ ...row, text: `${row.name}：${providerDetail(row.status)}` }] : [];
}));
const trafficTitle = computed(() =>
  `${trafficLabel.value}: ${providerRows.value.map(row => `${row.text} ${row.quota}`).join(" · ")}. ${t("widgetTitleSuffix")}`,
);
const manualWidgetSelection = computed<ManualWidgetSelection>(() => traffic.status?.manualOverride ?? "off");
const mochiVoiceStatusText = computed(() => {
  return [mochiVoiceFeedback.value, mochiVoiceConfig.value?.lastStatus].filter(Boolean).join(" · ");
});
const manualWidgetStateOptions = computed<Array<{ label: string; value: ManualWidgetSelection }>>(() => [
  { label: t("manualStateOff"), value: "off" },
  { label: t("manualStateReady"), value: "ready" },
  { label: t("manualStateRunning"), value: "running" },
  { label: t("manualStateWaiting"), value: "waiting" },
  { label: t("manualStateFailed"), value: "failed" },
  { label: t("manualStateComplete"), value: "complete" },
]);

async function refreshAll(showFeedback = false) {
  refreshAllLoading.value = true;
  if (showFeedback) refreshFeedback.value = t("refreshing");
  try {
    await Promise.all([
      dashboard.load(),
      events.load(),
      channels.load(),
      hooks.load(),
      loadClaudeProvider(),
      hooks.previewInstall(),
      approvals.load(),
      traffic.load(),
      loadAutostart(),
      loadPetConfig(),
      loadMochiVoiceConfig(),
    ]);
    if (showFeedback) refreshFeedback.value = t("refreshDone");
  } catch (error) {
    console.error("Notice refresh failed", error);
    if (showFeedback) refreshFeedback.value = t("refreshFailed");
  } finally {
    refreshAllLoading.value = false;
  }
}

async function refreshRuntimeStatus(showFeedback = false) {
  runtimeRefreshLoading.value = true;
  if (showFeedback) refreshFeedback.value = t("refreshing");
  try {
    await traffic.refreshRuntimeStatus();
    await Promise.all([dashboard.load(), events.load(), approvals.load()]);
    if (showFeedback) refreshFeedback.value = t("statusRefreshDone");
  } catch (error) {
    console.error("Notice runtime status refresh failed", error);
    if (showFeedback) refreshFeedback.value = t("statusRefreshFailed");
  } finally {
    runtimeRefreshLoading.value = false;
  }
}

async function loadLocale() {
  try {
    locale.value = await api.appLocale();
  } catch (error) {
    console.error("Notice locale load failed", error);
  }
}

async function setLocale(nextLocale: AppLocale) {
  locale.value = await api.setAppLocale(nextLocale);
}

async function loadAutostart() {
  autostartLoading.value = true;
  try {
    autostartEnabled.value = await api.autostartEnabled();
  } catch (error) {
    console.error("Notice autostart load failed", error);
  } finally {
    autostartLoading.value = false;
  }
}

async function setAutostart(enabled: boolean) {
  autostartLoading.value = true;
  try {
    autostartEnabled.value = await api.setAutostartEnabled(enabled);
  } catch (error) {
    console.error("Notice autostart update failed", error);
    autostartEnabled.value = await api.autostartEnabled().catch(() => autostartEnabled.value);
  } finally {
    autostartLoading.value = false;
  }
}

async function loadPetConfig() {
  petLoading.value = true;
  try {
    petConfig.value = await api.petConfig();
    petEnabled.value = petConfig.value.enabled;
    petBaseUrl.value = petConfig.value.baseUrl ?? "";
  } catch (error) {
    console.error("Notice pet config load failed", error);
  } finally {
    petLoading.value = false;
  }
}

async function savePetConfig() {
  petLoading.value = true;
  petFeedback.value = "";
  try {
    petConfig.value = await api.savePetConfig(petEnabled.value, petBaseUrl.value);
    petEnabled.value = petConfig.value.enabled;
    petBaseUrl.value = petConfig.value.baseUrl ?? "";
    petFeedback.value = t("petSaved");
  } catch (error) {
    console.error("Notice pet config save failed", error);
    petFeedback.value = String(error);
  } finally {
    petLoading.value = false;
  }
}

async function testPetConnection() {
  petTesting.value = true;
  petFeedback.value = "";
  try {
    await savePetConfig();
    const message = await api.testPetConnection();
    petConfig.value = await api.petConfig();
    petFeedback.value = message || t("petTestSent");
  } catch (error) {
    console.error("Notice pet test failed", error);
    petFeedback.value = String(error);
  } finally {
    petTesting.value = false;
  }
}

async function loadMochiVoiceConfig(showLoading = true) {
  if (showLoading) mochiVoiceLoading.value = true;
  try {
    mochiVoiceConfig.value = await api.mochiVoiceConfig();
    mochiVoiceEnabled.value = mochiVoiceConfig.value.enabled;
    mochiVoiceSerialPort.value = mochiVoiceConfig.value.serialPort;
    mochiVoiceAsrUrl.value = mochiVoiceConfig.value.asrUrl;
  } catch (error) {
    console.error("Notice Mochi voice config load failed", error);
  } finally {
    if (showLoading) mochiVoiceLoading.value = false;
  }
}

async function saveMochiVoiceConfig() {
  mochiVoiceLoading.value = true;
  mochiVoiceFeedback.value = "";
  try {
    mochiVoiceConfig.value = await api.saveMochiVoiceConfig(
      mochiVoiceEnabled.value,
      mochiVoiceSerialPort.value,
      mochiVoiceAsrUrl.value,
    );
    mochiVoiceEnabled.value = mochiVoiceConfig.value.enabled;
    mochiVoiceSerialPort.value = mochiVoiceConfig.value.serialPort;
    mochiVoiceAsrUrl.value = mochiVoiceConfig.value.asrUrl;
    mochiVoiceFeedback.value = t("mochiVoiceSaved");
    window.setTimeout(() => {
      if (mochiVoiceFeedback.value === t("mochiVoiceSaved")) mochiVoiceFeedback.value = "";
    }, 1500);
  } catch (error) {
    console.error("Notice Mochi voice config save failed", error);
    mochiVoiceFeedback.value = String(error);
  } finally {
    mochiVoiceLoading.value = false;
  }
}

async function setManualWidgetSelection(value: ManualWidgetSelection) {
  manualWidgetFeedback.value = "";
  try {
    await traffic.setManualOverride(value === "off" ? undefined : value);
    manualWidgetFeedback.value = t("manualWidgetSaved");
  } catch (error) {
    console.error("Notice manual widget state update failed", error);
    manualWidgetFeedback.value = String(error);
  }
}

async function closeTrafficWidget() {
  await traffic.setEnabled(false);
}

async function toggleTrafficAlwaysOnTop() {
  await traffic.setAlwaysOnTop(!(traffic.status?.alwaysOnTop ?? true));
}

function startWidgetDrag(event: MouseEvent) {
  if (event.button !== 0) return;
  void getCurrentWindow().startDragging().catch((error) => {
    console.error("Notice widget drag failed", error);
  });
}

async function openWidgetMenu(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
  const menu = await Menu.new({
    items: [
      {
        id: "refresh-runtime-status",
        text: t("refreshStatus"),
        action: () => {
          void refreshRuntimeStatus();
        },
      },
      {
        id: "toggle-always-on-top",
        text: traffic.status?.alwaysOnTop === false ? "置顶显示" : "取消置顶",
        action: () => {
          void toggleTrafficAlwaysOnTop();
        },
      },
      {
        id: "hide-traffic-widget",
        text: "隐藏小组件",
        action: () => {
          void closeTrafficWidget();
        },
      },
    ],
  });
  await menu.popup(undefined, getCurrentWindow());
}

onMounted(async () => {
  await loadLocale();
  if (isWidget) {
    document.documentElement.classList.add("widget-root");
    document.body.classList.add("widget-body");
    await getCurrentWindow().setShadow(false).catch((error) => {
      console.error("Notice widget shadow update failed", error);
    });
    await traffic.load();
    refreshTimer = window.setInterval(() => traffic.load(), 800);
    return;
  }
  await refreshAll();
  refreshTimer = window.setInterval(() => {
    void traffic.load();
    if (active.value === "providers") void loadClaudeProvider();
  }, 4000);
  mochiVoiceStatusTimer = window.setInterval(() => {
    if (active.value === "settings" && mochiVoiceEnabled.value) {
      void loadMochiVoiceConfig(false);
    }
  }, 1000);
});

onBeforeUnmount(() => {
  if (refreshTimer) window.clearInterval(refreshTimer);
  if (mochiVoiceStatusTimer) window.clearInterval(mochiVoiceStatusTimer);
  document.documentElement.classList.remove("widget-root");
  document.body.classList.remove("widget-body");
});

watch(active, async (value) => {
  if (value === "channels") await channels.load();
  if (value === "events") await events.load();
  if (value === "providers") {
    await hooks.load();
    await hooks.previewInstall();
  }
  if (value === "approvals") await approvals.load();
  if (value === "dashboard") await dashboard.load();
  if (value === "settings") {
    await Promise.all([traffic.load(), loadAutostart(), loadPetConfig(), loadMochiVoiceConfig()]);
  }
});
</script>

<template>
  <n-config-provider :theme="darkTheme" :locale="naiveLocale" :date-locale="naiveDateLocale">
    <n-message-provider>
      <n-dialog-provider>
        <div
          v-if="isWidget"
          :class="trafficClass"
          role="button"
          data-tauri-drag-region
          :title="trafficTitle"
          @mousedown="startWidgetDrag"
          @contextmenu="openWidgetMenu"
        >
          <span class="traffic-housing" data-tauri-drag-region>
            <span class="traffic-light red" data-tauri-drag-region />
            <span class="traffic-light yellow" data-tauri-drag-region />
            <span class="traffic-light green" data-tauri-drag-region />
          </span>
          <span class="traffic-copy" data-tauri-drag-region>
            <strong data-tauri-drag-region>{{ trafficLabel }}</strong>
            <div v-for="row in providerRows" :key="row.key" class="traffic-session" data-tauri-drag-region>
              <span class="traffic-status-text" :title="row.text" data-tauri-drag-region>{{ row.text }}</span>
              <span v-if="row.quota" class="traffic-weekly" :title="`${row.name} 7 天剩余额度`" data-tauri-drag-region>{{ row.quota }}</span>
            </div>
          </span>
        </div>

        <n-layout v-else has-sider class="app-shell">
          <n-layout-sider width="220" class="sidebar">
            <div class="brand">
              <h1 class="brand-title">Notice</h1>
              <p class="brand-subtitle">{{ t("brandSubtitle") }}</p>
            </div>
            <n-menu v-model:value="active" :options="menuOptions" />
          </n-layout-sider>

          <n-layout-content class="content">
            <h2 class="section-title">{{ currentTitle }}</h2>

            <section v-if="active === 'dashboard'">
              <div class="grid">
                <n-card><n-statistic :label="t('service')" :value="dashboard.summary?.serviceStatus ?? 'loading'" /></n-card>
                <n-card><n-statistic :label="t('today')" :value="dashboard.summary?.todayTotal ?? 0" /></n-card>
                <n-card><n-statistic :label="t('success')" :value="dashboard.summary?.todaySuccess ?? 0" /></n-card>
                <n-card><n-statistic :label="t('failure')" :value="dashboard.summary?.todayFailure ?? 0" /></n-card>
              </div>
              <n-card class="panel" style="margin-top: 16px">
                <template #header>{{ t("trafficWidget") }}</template>
                <div class="setting-row">
                  <div>
                    <strong>{{ trafficLabel }}</strong>
                    <p v-for="row in providerRows" :key="row.key">{{ row.text }}<span v-if="row.quota" class="dashboard-weekly">{{ row.quota }}</span></p>
                  </div>
                  <n-switch
                    :value="traffic.status?.enabled ?? true"
                    :loading="traffic.loading"
                    @update:value="traffic.setEnabled"
                  />
                </div>
              </n-card>
              <n-card class="panel" style="margin-top: 16px">
                <template #header>{{ t("recentSummary") }}</template>
                {{ dashboard.summary?.recentSummary ?? t("noSummary") }}
              </n-card>
            </section>

            <section v-if="active === 'events'">
              <div class="toolbar">
                <n-input v-model:value="events.filter.search" :placeholder="t('searchPlaceholder')" clearable />
                <n-input v-model:value="events.filter.level" :placeholder="t('level')" clearable />
                <n-input v-model:value="events.filter.project" :placeholder="t('project')" clearable />
                <n-button @click="events.load">{{ t("search") }}</n-button>
                <n-button type="error" @click="events.clear">{{ t("clear") }}</n-button>
              </div>
              <n-data-table :columns="eventColumns" :data="events.items" :loading="events.loading" :bordered="false" />
            </section>

            <section v-if="active === 'channels'">
              <n-card class="panel">
                <template #header>{{ t("feishu") }}</template>
                <div class="setting-row compact">
                  <div>
                    <strong>{{ t("feishuNotifications") }}</strong>
                    <p>{{ channels.config?.enabled === false ? t("feishuDisabled") : t("feishuEnabled") }}</p>
                  </div>
                  <n-switch
                    :value="channels.config?.enabled ?? true"
                    @update:value="channels.setEnabled"
                  />
                </div>
                <div class="saved-config">
                  <n-tag :type="channels.config?.hasWebhook ? 'success' : 'warning'">
                    {{ channels.config?.hasWebhook ? t("webhookSaved") : t("webhookMissing") }}
                  </n-tag>
                  <span>{{ channels.config?.webhookMasked ?? t("webhookPrompt") }}</span>
                </div>
                <n-form label-placement="top">
                  <n-form-item :label="t('webhookUrl')">
                    <n-input
                      v-model:value="channels.webhookUrl"
                      type="password"
                      show-password-on="click"
                      :placeholder="channels.config?.hasWebhook ? t('webhookOverwrite') : t('webhookPaste')"
                    />
                  </n-form-item>
                  <n-form-item :label="t('signSecret')">
                    <n-input
                      v-model:value="channels.signSecret"
                      type="password"
                      show-password-on="click"
                      :placeholder="channels.config?.hasSignSecret ? t('signSecretOverwrite') : t('signSecretOptional')"
                    />
                  </n-form-item>
                  <n-space>
                    <n-button type="primary" :loading="channels.saving" @click="channels.save()">{{ t("save") }}</n-button>
                    <n-button :disabled="channels.config?.enabled === false" :loading="channels.testing" @click="channels.test()">{{ t("sendTest") }}</n-button>
                  </n-space>
                </n-form>
              </n-card>
              <n-card>
                <p>Webhook: {{ channels.config?.webhookMasked ?? t("notConfigured") }}</p>
                <p>{{ t("signSecret") }}: {{ channels.config?.hasSignSecret ? t("configured") : t("notConfigured") }}</p>
                <p>{{ t("lastStatus") }}: {{ channels.config?.lastStatus ?? channels.lastMessage }}</p>
                <n-tag v-if="channels.lastMessage" type="success">{{ channels.lastMessage }}</n-tag>
                <n-tag v-if="channels.error" type="error">{{ channels.error }}</n-tag>
              </n-card>
            </section>

            <section v-if="active === 'providers'">
              <n-card class="panel">
                <template #header>{{ t("hookManager") }}</template>
                <p>{{ t("config") }}: {{ hooks.status?.configPath }}</p>
                <p>{{ t("status") }}: {{ hooks.status?.message }}</p>
                <n-space>
                  <n-button @click="hooks.previewInstall">{{ t("preview") }}</n-button>
                  <n-button type="primary" @click="hooks.install">{{ t("install") }}</n-button>
                  <n-button type="warning" @click="hooks.uninstall">{{ t("uninstall") }}</n-button>
                </n-space>
              </n-card>
              <pre class="code-preview">{{ hooks.preview?.preview }}</pre>
              <n-card class="panel" style="margin-top: 16px">
                <template #header>{{ t("claudeHookManager") }}</template>
                <p>{{ t("claudeHookDescription") }}</p>
                <p>{{ t("config") }}: {{ claudeHooks?.configPath }}</p>
                <p>{{ t("status") }}: {{ claudeHooks?.message }}</p>
                <n-space>
                  <n-button :disabled="claudeBusy" @click="claudeAction('preview')">{{ t("preview") }}</n-button>
                  <n-button type="primary" :disabled="claudeBusy" @click="claudeAction('install')">{{ t("install") }}</n-button>
                  <n-button type="warning" :disabled="claudeBusy" @click="claudeAction('uninstall')">{{ t("uninstall") }}</n-button>
                </n-space>
                <pre v-if="claudeHookPreview" class="code-preview">{{ claudeHookPreview.preview }}</pre>
                <h3>{{ t("claudeQuotaTitle") }}</h3>
                <p>{{ t("claudeQuotaDescription") }}</p>
                <p>{{ claudeUsage?.message }}</p>
                <p v-if="claudeUsage?.remainingPercent != null">{{ t("claudeQuotaRemaining") }}：{{ quotaText(claudeUsage.remainingPercent) }}</p>
                <n-space>
                  <n-button :disabled="claudeBusy" @click="claudeAction('login')">{{ t("claudeSignIn") }}</n-button>
                  <n-button :loading="claudeBusy" @click="claudeAction('refresh')">{{ t("claudeQuotaRefresh") }}</n-button>
                </n-space>
                <p v-if="claudeError" class="level-error">{{ claudeError }}</p>
              </n-card>
            </section>

            <section v-if="active === 'approvals'">
              <n-data-table :columns="approvalColumns" :data="approvals.items" :bordered="false" />
            </section>

            <section v-if="active === 'settings'">
              <n-card>
                <template #header>{{ t("runtime") }}</template>
                <div class="setting-row language-row">
                  <div>
                    <strong>{{ t("language") }}</strong>
                    <p>{{ t("languageDescription") }} {{ t("currentLanguage") }}: {{ locale === "zh-CN" ? t("simplifiedChinese") : t("english") }}</p>
                  </div>
                  <n-button type="primary" @click="setLocale(locale === 'zh-CN' ? 'en' : 'zh-CN')">
                    {{ locale === "zh-CN" ? t("switchToEnglish") : t("switchToChinese") }}
                  </n-button>
                </div>
                <div class="setting-row">
                  <div>
                    <strong>{{ t("trafficWidget") }}</strong>
                    <p>{{ t("trafficDescription") }}</p>
                  </div>
                  <n-switch
                    :value="traffic.status?.enabled ?? true"
                    :loading="traffic.loading"
                    @update:value="traffic.setEnabled"
                  />
                </div>
                <div class="setting-block manual-state-block">
                  <div>
                    <strong>{{ t("manualWidgetOverride") }}</strong>
                    <p>{{ t("manualWidgetOverrideDescription") }}</p>
                  </div>
                  <div class="manual-state-buttons" aria-label="Photo status override">
                    <n-button
                      v-for="option in manualWidgetStateOptions"
                      :key="option.value"
                      size="small"
                      :type="manualWidgetSelection === option.value ? 'primary' : 'default'"
                      :secondary="manualWidgetSelection !== option.value"
                      :loading="traffic.loading && manualWidgetSelection === option.value"
                      @click="setManualWidgetSelection(option.value)"
                    >
                      {{ option.label }}
                    </n-button>
                  </div>
                  <p v-if="manualWidgetFeedback" class="refresh-feedback">{{ manualWidgetFeedback }}</p>
                </div>
                <div class="setting-block">
                  <div class="setting-row">
                    <div>
                      <strong>{{ t("petIntegration") }}</strong>
                      <p>{{ t("petDescription") }}</p>
                    </div>
                    <n-switch v-model:value="petEnabled" :loading="petLoading" @update:value="savePetConfig" />
                  </div>
                  <n-form label-placement="top">
                    <n-form-item :label="t('petAddress')">
                      <n-input
                        v-model:value="petBaseUrl"
                        :placeholder="t('petAddressPlaceholder')"
                        @keyup.enter="savePetConfig"
                      />
                    </n-form-item>
                    <n-space align="center">
                      <n-button type="primary" :loading="petLoading" @click="savePetConfig">{{ t("save") }}</n-button>
                      <n-button secondary :loading="petTesting" @click="testPetConnection">{{ t("testPet") }}</n-button>
                      <span class="refresh-feedback">{{ petFeedback || petConfig?.lastStatus }}</span>
                    </n-space>
                  </n-form>
                </div>
                <div class="setting-block">
                  <div class="setting-row">
                    <div>
                      <strong>{{ t("mochiVoiceInput") }}</strong>
                      <p>{{ t("mochiVoiceDescription") }}</p>
                    </div>
                    <n-switch
                      v-model:value="mochiVoiceEnabled"
                      :loading="mochiVoiceLoading"
                      @update:value="saveMochiVoiceConfig"
                    />
                  </div>
                  <n-form label-placement="top">
                    <n-form-item :label="t('mochiVoiceSerialPort')">
                      <n-input
                        v-model:value="mochiVoiceSerialPort"
                        :placeholder="t('mochiVoiceSerialPlaceholder')"
                        @keyup.enter="saveMochiVoiceConfig"
                      />
                    </n-form-item>
                    <n-form-item :label="t('mochiVoiceAsrUrl')">
                      <n-input
                        v-model:value="mochiVoiceAsrUrl"
                        :placeholder="t('mochiVoiceAsrPlaceholder')"
                        @keyup.enter="saveMochiVoiceConfig"
                      />
                    </n-form-item>
                    <n-space align="center">
                      <n-button type="primary" :loading="mochiVoiceLoading" @click="saveMochiVoiceConfig">
                        {{ t("save") }}
                      </n-button>
                      <span class="refresh-feedback">
                        {{ mochiVoiceStatusText }}
                      </span>
                    </n-space>
                  </n-form>
                </div>
                <div class="setting-row">
                  <div>
                    <strong>{{ t("autostart") }}</strong>
                    <p>{{ t("autostartDescription") }}</p>
                  </div>
                  <n-switch
                    :value="autostartEnabled"
                    :loading="autostartLoading"
                    @update:value="setAutostart"
                  />
                </div>
                <p>{{ t("localServer") }}: 127.0.0.1:3746</p>
                <p>{{ t("retention") }}</p>
                <p>{{ t("criticalTimeout") }}</p>
                <n-space align="center">
                  <n-button :loading="refreshAllLoading" @click="() => refreshAll(true)">{{ t("refreshAll") }}</n-button>
                  <n-button secondary :loading="runtimeRefreshLoading" @click="() => refreshRuntimeStatus(true)">
                    {{ t("refreshStatus") }}
                  </n-button>
                  <span class="refresh-feedback">{{ refreshFeedback }}</span>
                </n-space>
              </n-card>
            </section>
          </n-layout-content>
        </n-layout>
      </n-dialog-provider>
    </n-message-provider>
  </n-config-provider>
</template>

<style scoped>
.level {
  padding: 3px 8px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 700;
}

.level-info {
  background: #1d3342;
  color: #8bd4ff;
}

.level-success {
  background: #173727;
  color: #89e7b1;
}

.level-warning {
  background: #423817;
  color: #f2d477;
}

.level-error {
  background: #461f24;
  color: #ff9aa8;
}

.row-actions {
  display: flex;
  gap: 8px;
}

.saved-config {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 16px;
  color: #b9c7c2;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
}

.setting-row.compact {
  margin-bottom: 18px;
  padding-bottom: 14px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.setting-row.language-row {
  margin-bottom: 18px;
  padding-bottom: 16px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.setting-row strong {
  display: block;
  color: #eef2f0;
  font-size: 14px;
}

.setting-row p {
  margin: 4px 0 0;
  color: #8ca39b;
}

.manual-state-block {
  margin: 18px 0;
  display: grid;
  gap: 10px;
}

.manual-state-block strong {
  display: block;
  color: #eef2f0;
  font-size: 14px;
}

.manual-state-block p {
  margin: 4px 0 0;
  color: #8ca39b;
}

.manual-state-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.manual-state-buttons .n-button {
  min-width: 76px;
}

.refresh-feedback {
  min-width: 84px;
  color: #8ca39b;
  font-size: 13px;
}

.traffic-widget {
  position: relative;
  width: 224px;
  height: 68px;
  margin: 4px;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 8px 10px;
  border: 1px solid transparent;
  border-radius: 18px;
  background:
    linear-gradient(180deg, rgba(45, 49, 51, 0.88), rgba(20, 22, 24, 0.86)),
    rgba(19, 21, 23, 0.82);
  color: #eef2f0;
  box-shadow: inset 0 1px rgba(255, 255, 255, 0.18);
  backdrop-filter: blur(18px) saturate(1.4);
  -webkit-backdrop-filter: blur(18px) saturate(1.4);
  cursor: pointer;
  overflow: hidden;
  user-select: none;
}

.traffic-widget:hover {
  border-color: transparent;
}

.traffic-housing {
  width: 26px;
  height: 42px;
  flex-shrink: 0;
  display: grid;
  justify-items: center;
  align-content: center;
  gap: 4px;
  padding: 5px;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.34);
  box-shadow: inset 0 1px 4px rgba(0, 0, 0, 0.56);
}

.traffic-light {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: #4a4d50;
  opacity: 0.52;
  box-shadow: inset 0 0 2px rgba(0, 0, 0, 0.8);
}

.traffic-red .traffic-light.red {
  background: #ff5f57;
  opacity: 1;
  animation: trafficPulseRed 1.15s ease-in-out infinite;
  box-shadow: 0 0 10px rgba(255, 95, 87, 0.95);
}

.traffic-yellow .traffic-light.yellow {
  background: #ffbd2e;
  opacity: 1;
  animation: trafficPulseYellow 1.4s ease-in-out infinite;
  box-shadow: 0 0 10px rgba(255, 189, 46, 0.95);
}

.traffic-green .traffic-light.green {
  background: #28c840;
  opacity: 1;
  animation: trafficBreathe 2.4s ease-in-out infinite;
  box-shadow: 0 0 10px rgba(40, 200, 64, 0.85);
}

.traffic-running .traffic-light.red,
.traffic-running .traffic-light.yellow,
.traffic-running .traffic-light.green {
  opacity: 0.38;
  animation: trafficMarquee 1.05s linear infinite;
}

.traffic-running .traffic-light.red {
  background: #ff5f57;
  box-shadow: 0 0 8px rgba(255, 95, 87, 0.7);
}

.traffic-running .traffic-light.yellow {
  background: #ffbd2e;
  animation-delay: 0.35s;
  box-shadow: 0 0 8px rgba(255, 189, 46, 0.7);
}

.traffic-running .traffic-light.green {
  background: #28c840;
  animation-delay: 0.7s;
  box-shadow: 0 0 8px rgba(40, 200, 64, 0.7);
}

.traffic-copy {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-align: left;
}

.traffic-copy strong {
  max-width: 172px;
  overflow: hidden;
  color: #f7faf8;
  font-size: 12px;
  font-weight: 700;
  line-height: 1.1;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.traffic-copy .traffic-session {
  max-width: 172px;
  overflow: hidden;
  color: #aeb9b5;
  font-size: 10px;
  line-height: 1.2;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.traffic-copy .traffic-session {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: 8px;
  max-width: none;
  font-variant-numeric: tabular-nums;
}

.traffic-copy .traffic-status-text,
.traffic-copy .traffic-weekly {
  font-family: -apple-system, BlinkMacSystemFont, "PingFang SC", sans-serif;
  font-size: 10px;
  font-weight: 400;
  line-height: 12px;
  letter-spacing: 0;
  -webkit-text-size-adjust: none;
  text-size-adjust: none;
}

.traffic-status-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.traffic-weekly {
  flex-shrink: 0;
  color: inherit;
}

.dashboard-weekly {
  margin-left: 24px;
  font-variant-numeric: tabular-nums;
}

@keyframes trafficPulseRed {
  0%,
  100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.22);
  }
}

@keyframes trafficPulseYellow {
  0%,
  100% {
    filter: brightness(0.92);
  }
  50% {
    filter: brightness(1.28);
  }
}

@keyframes trafficBreathe {
  0%,
  100% {
    opacity: 0.82;
  }
  50% {
    opacity: 1;
  }
}

@keyframes trafficMarquee {
  0%,
  100% {
    opacity: 0.34;
    transform: scale(0.92);
    filter: brightness(0.75);
  }
  18%,
  42% {
    opacity: 1;
    transform: scale(1.24);
    filter: brightness(1.35);
  }
}

.plain-button {
  min-width: 76px;
  height: 30px;
  border: 0;
  border-radius: 6px;
  color: white;
  cursor: pointer;
}

.approve {
  background: #1c7a4b;
}

.reject {
  background: #9e2f3e;
}
</style>
