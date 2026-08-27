import { invoke } from "@tauri-apps/api/core";
import type { ActionPreview, ActionResult, CurrentStatus } from "./types";

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
    processes: [
      { pid: 4242, startedAt: Date.now() / 1000 - 3600, name: "chrome.exe", cpuPercent: 8.2, memoryBytes: 1_610_612_736, isCritical: false },
      { pid: 2333, startedAt: Date.now() / 1000 - 7200, name: "Code.exe", cpuPercent: 3.4, memoryBytes: 845_152_256, isCritical: false }
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

export async function confirmAction(previewId: string): Promise<ActionResult> {
  return invoke("confirm_action", { previewId });
}
