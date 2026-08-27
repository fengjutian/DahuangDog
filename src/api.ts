import { invoke } from "@tauri-apps/api/core";
import type { ActionPreview, ActionResult, AppUsageRecord, AppUsageSummary, CurrentStatus, HistorySummary, LocalDiagnosis, SecurityReport, UserSettings } from "./types";

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

export async function diagnosePerformance(): Promise<LocalDiagnosis> {
  if (isTauri()) return invoke("diagnose_performance");
  return {
    summary: "我看了一圈，当前没有明显的资源瓶颈。",
    details: ["CPU 当前使用率 18%", "内存当前使用率 52%", "chrome.exe：CPU 8.2%，内存 1.5 GB"],
    suggestions: ["如果卡顿再次出现，我会继续记录当时的状态"],
    confidence: "medium"
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

const demoSettings: UserSettings = { cpuThreshold: 90, memoryThreshold: 90, samplingSeconds: 2, lowPowerMode: false, notificationsEnabled: true, retentionDays: 7 };

export async function getSettings(): Promise<UserSettings> {
  return isTauri() ? invoke("get_settings") : demoSettings;
}

export async function saveSettings(settings: UserSettings): Promise<UserSettings> {
  return isTauri() ? invoke("update_settings", { settings }) : settings;
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
