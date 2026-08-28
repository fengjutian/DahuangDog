import { Channel, invoke } from "@tauri-apps/api/core";
import type { ActionPreview, ActionResult, AiStatus, AlertRecord, AppUsageRecord, AppUsageSummary, ApplicationHistory, CleanupReport, CurrentStatus, HistorySummary, LocalDiagnosis, MaintenancePreview, PeriodicPattern, SecurityReport, StorageCapacitySnapshot, StorageScanResult, UserSettings } from "./types";

const demoStatus: CurrentStatus = {
  dogState: "patrol",
  summary: "我正在巡逻，一切看起来都好。",
  healthScore: 96,
  snapshot: {
    capturedAt: Date.now(),
    cpuPercent: 18,
    memoryPercent: 52,
    usedMemoryBytes: 8_934_211_584,
    totalMemoryBytes: 17_179_869_184,
    diskReadBps: 1_240_000,
    diskWriteBps: 420_000,
    networkReceiveBps: 2_850_000,
    networkSendBps: 310_000,
    diskTotalBytes: 512_000_000_000,
    diskAvailableBytes: 180_000_000_000,
    uptimeSeconds: 86400,
    hardware: { cpuCores: [{ name: "核心 1", usagePercent: 18, frequencyMhz: 3200 }], gpus: [], battery: null, temperatures: [], fans: [], disks: [
      { name: "系统", mountPoint: "C:\\", totalBytes: 512 * 1024 ** 3, availableBytes: 126 * 1024 ** 3, readBps: 1_240_000, writeBps: 420_000 },
      { name: "数据", mountPoint: "D:\\", totalBytes: 1024 * 1024 ** 3, availableBytes: 684 * 1024 ** 3, readBps: 480_000, writeBps: 160_000 }
    ], networks: [], gpuStatus: "演示模式", fanStatus: "演示模式", appNetworkStatus: "演示模式" },
    processes: [
      { pid: 4242, parentPid: null, startedAt: Date.now() / 1000 - 3600, name: "chrome.exe", cpuPercent: 8.2, memoryBytes: 1_610_612_736, isCritical: false },
      { pid: 4243, parentPid: 4242, startedAt: Date.now() / 1000 - 3500, name: "chrome.exe", cpuPercent: 2.1, memoryBytes: 410_612_736, isCritical: false },
      { pid: 2333, parentPid: null, startedAt: Date.now() / 1000 - 7200, name: "Code.exe", cpuPercent: 3.4, memoryBytes: 845_152_256, isCritical: false }
    ],
    applications: [
      { rootPid: 4242, name: "chrome.exe", memberCount: 2, cpuPercent: 10.3, memoryBytes: 2_021_225_472, rootProcess: { pid: 4242, parentPid: null, startedAt: Date.now() / 1000 - 3600, name: "chrome.exe", cpuPercent: 8.2, memoryBytes: 1_610_612_736, isCritical: false }, members: [] },
      { rootPid: 2333, name: "Code.exe", memberCount: 1, cpuPercent: 3.4, memoryBytes: 845_152_256, rootProcess: { pid: 2333, parentPid: null, startedAt: Date.now() / 1000 - 7200, name: "Code.exe", cpuPercent: 3.4, memoryBytes: 845_152_256, isCritical: false }, members: [] }
    ]
  },
  findings: [],
  timeline: [{ id: "demo", occurredAt: Date.now(), kind: "patrol", message: "开始巡逻，系统状态正常" }],
  verification: null
};

function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export async function getCurrentStatus(): Promise<CurrentStatus> {
  return isTauri() ? invoke("get_current_status") : demoStatus;
}

export async function scanStorageTree(root: string, taskId: string, force: boolean, onEntry: (entry: StorageScanResult["root"]) => void): Promise<StorageScanResult> {
  if (!isTauri()) throw new Error("浏览器预览模式无法扫描本机磁盘，请在桌面应用中使用");
  const channel = new Channel<StorageScanResult["root"]>();
  channel.onmessage = onEntry;
  return invoke("scan_storage_tree", { root, taskId, force, onEntry: channel });
}

export async function cancelStorageScan(taskId: string): Promise<boolean> {
  if (!isTauri()) return false;
  return invoke("cancel_storage_scan", { taskId });
}

export async function listStorageDirectory(root: string, force = false): Promise<StorageScanResult> {
  if (!isTauri()) throw new Error("浏览器预览模式无法读取本机目录，请在桌面应用中使用");
  return invoke("list_storage_directory", { root, force });
}

export async function getStorageCapacityHistory(root: string): Promise<StorageCapacitySnapshot[]> {
  if (!isTauri()) return [];
  return invoke("get_storage_capacity_history", { root });
}

export async function prepareTerminate(pid: number, startedAt: number): Promise<ActionPreview> {
  if (!isTauri()) throw new Error("浏览器预览模式不能执行系统操作");
  return invoke("prepare_terminate_process", { pid, startedAt });
}

export async function preparePriority(pid: number, startedAt: number, level: "belowNormal" | "normal" | "aboveNormal"): Promise<ActionPreview> {
  if (!isTauri()) throw new Error("浏览器预览模式不能执行系统操作");
  return invoke("prepare_process_priority", { pid, startedAt, level });
}

export async function openProcessLocation(pid: number, startedAt: number): Promise<ActionResult> {
  if (!isTauri()) throw new Error("浏览器预览模式不能执行系统操作");
  return invoke("open_process_location", { pid, startedAt });
}

export async function confirmAction(previewId: string): Promise<ActionResult> {
  return invoke("confirm_action", { previewId });
}

export async function getHistory(): Promise<HistorySummary> {
  if (isTauri()) return invoke("get_history");
  const now = Date.now();
  return {
    points: Array.from({ length: 40 }, (_, index) => ({
      capturedAt: now - (39 - index) * 2000,
      cpuPercent: 20 + Math.sin(index / 4) * 8,
      memoryPercent: 50 + index * 0.08,
      diskBps: 800_000 + Math.abs(Math.sin(index)) * 2_000_000,
      networkBps: 500_000 + Math.abs(Math.cos(index / 2)) * 3_000_000
    })),
    baselineCpuPercent: 24,
    baselineMemoryPercent: 51,
    peakCpuPercent: 31,
    peakMemoryPercent: 54,
    averageDiskBps: 1_800_000,
    averageNetworkBps: 2_100_000
  };
}

export async function getHistoryRange(rangeMinutes: number): Promise<HistorySummary> {
  return isTauri() ? invoke("get_history_range", { rangeMinutes }) : getHistory();
}

export async function getApplicationHistory(name: string, rangeMinutes: number): Promise<ApplicationHistory> {
  if (isTauri()) return invoke("get_application_history", { name, rangeMinutes });
  const history = await getHistory();
  return { name, rangeMinutes, points: history.points.map(point => ({ capturedAt: point.capturedAt, cpuPercent: point.cpuPercent / 2, memoryBytes: 1_500_000_000 + point.memoryPercent * 1_000_000, diskReadBps: point.diskBps * .7, diskWriteBps: point.diskBps * .3, networkBps: null })) };
}

export async function getAlerts(status?: string): Promise<AlertRecord[]> {
  return isTauri() ? invoke("get_alerts", { status: status || null }) : [];
}

export async function updateAlert(id: string, status: AlertRecord["status"], note: string): Promise<void> {
  if (!isTauri()) return;
  return invoke("update_alert", { id, status, note });
}

export async function getPeriodicPatterns(days = 30): Promise<PeriodicPattern[]> {
  return isTauri() ? invoke("get_periodic_patterns", { days }) : [];
}

export async function scanCleanup(): Promise<CleanupReport> {
  if (!isTauri()) return { scannedAt: Date.now(), reclaimableBytes: 0, candidates: [] };
  return invoke("scan_cleanup");
}
export async function prepareCleanup(paths: string[]): Promise<MaintenancePreview> { return invoke("prepare_cleanup", { paths }); }
export async function prepareStartupChange(source: string, name: string, command: string, enable: boolean): Promise<MaintenancePreview> { return invoke("prepare_startup_change", { source, name, command, enable }); }
export async function confirmMaintenance(previewId: string): Promise<ActionResult> { return invoke("confirm_maintenance", { previewId }); }

export async function diagnosePerformance(): Promise<LocalDiagnosis> {
  if (isTauri()) return invoke("diagnose_performance");
  return {
    summary: "我看了一圈，当前没有明显的资源瓶颈。",
    details: ["CPU 当前使用率 18%", "内存当前使用率 52%", "chrome.exe：CPU 8.2%，内存 1.5 GB"],
    suggestions: ["如果卡顿再次出现，我会继续记录当时的状态"],
    confidence: "medium",
    source: "local",
    model: null
  };
}

export async function getSecurityReport(): Promise<SecurityReport> {
  if (!isTauri()) return {
    scannedAt: Date.now(), scannedPrograms: 42, signedPrograms: 38,
    summary: "发现 1 个值得进一步确认的项目。", securityScore: 86, mediumRiskCount: 1, lowRiskCount: 0,
    networkConnections: [], scheduledTasks: [], windowsServices: [],
    programs: [{ pid: 8842, name: "example.exe", path: "C:\\Users\\demo\\AppData\\Local\\example.exe", signatureStatus: "unverified", riskLevel: "medium", reasons: ["没有可验证的数字签名", "程序位于用户可写目录"] }],
    startupEntries: [{ name: "OneDrive", command: "OneDrive.exe /background", source: "当前用户\\Run", riskLevel: "normal", reasons: [] }]
  };
  return invoke("get_security_report");
}

export async function openFileLocation(path: string): Promise<ActionResult> {
  if (!isTauri()) throw new Error("浏览器预览模式不能打开本机文件位置");
  return invoke("open_file_location", { path });
}

export async function exportUsageCsv(content: string): Promise<ActionResult> {
  if (!isTauri()) throw new Error("浏览器预览模式不能写入下载目录");
  return invoke("export_usage_csv", { content });
}

const demoSettings: UserSettings = { cpuThreshold: 90, memoryThreshold: 90, samplingSeconds: 2, lowPowerMode: false, notificationsEnabled: true, retentionDays: 7, autoStart: false, applicationNetworkMonitoring: false, minimaxEnabled: false, minimaxModel: "MiniMax-M2.7", companionPersonality: "warm", companionQuietMode: false, reduceCompanionMotion: false };
let demoAiConfigured = false;

export async function getSettings(): Promise<UserSettings> {
  return isTauri() ? invoke("get_settings") : demoSettings;
}

export async function saveSettings(settings: UserSettings): Promise<UserSettings> {
  return isTauri() ? invoke("update_settings", { settings }) : settings;
}

export async function getAiStatus(): Promise<AiStatus> {
  return isTauri() ? invoke("get_ai_status") : { configured: demoAiConfigured };
}

export async function saveMinimaxApiKey(apiKey: string): Promise<AiStatus> {
  if (isTauri()) return invoke("save_minimax_api_key", { apiKey });
  demoAiConfigured = Boolean(apiKey.trim());
  return { configured: demoAiConfigured };
}

export async function clearMinimaxApiKey(): Promise<AiStatus> {
  if (isTauri()) return invoke("clear_minimax_api_key");
  demoAiConfigured = false;
  return { configured: false };
}

export async function testMinimaxConnection(model: string, apiKey: string): Promise<string> {
  if (!isTauri()) return "连接成功 · 浏览器演示模式";
  return invoke("test_minimax_connection", { model, apiKey: apiKey.trim() || null });
}

export async function clearLocalMemory(): Promise<void> {
  if (!isTauri()) throw new Error("浏览器预览模式没有本地记忆");
  return invoke("clear_local_memory");
}

export async function getAppUsageHistory(): Promise<AppUsageRecord[]> {
  if (isTauri()) return invoke("get_app_usage_history");
  const now = Date.now();
  return [{ sessionId: "demo", name: "chrome.exe", rootPid: 4242, startedAt: now - 7_200_000, firstSeenAt: now - 3_600_000, lastSeenAt: now, closedAt: null, runtimeSeconds: 7200, foregroundSeconds: 1840, backgroundSeconds: 5360, memberPeak: 8, isRunning: true }];
}

export async function getAppUsageSummary(periodDays = 7): Promise<AppUsageSummary> {
  if (isTauri()) return invoke("get_app_usage_summary", { periodDays });
  return {
    periodDays, applicationCount: 3, sessionCount: 6, totalRuntimeSeconds: 28800,
    totalForegroundSeconds: 10800, totalBackgroundSeconds: 18000, longestUsedApp: "chrome.exe",
    topApps: [{ name: "chrome.exe", sessionCount: 2, runtimeSeconds: 14400, foregroundSeconds: 7200, backgroundSeconds: 7200 }],
    dailyUsage: [{ date: new Date().toISOString().slice(0, 10), foregroundSeconds: 7200, launchCount: 2 }]
  };
}
