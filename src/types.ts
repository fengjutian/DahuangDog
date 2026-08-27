export type DogState =
  | "idle"
  | "patrol"
  | "suspicious"
  | "investigating"
  | "awaitingApproval"
  | "verifying"
  | "resolved";

export interface ProcessSample {
  pid: number;
  startedAt: number;
  name: string;
  cpuPercent: number;
  memoryBytes: number;
  isCritical: boolean;
}

export interface SystemSnapshot {
  capturedAt: number;
  cpuPercent: number;
  memoryPercent: number;
  usedMemoryBytes: number;
  totalMemoryBytes: number;
  diskReadBps: number;
  diskWriteBps: number;
  networkReceiveBps: number;
  networkSendBps: number;
  processes: ProcessSample[];
}

export interface Finding {
  id: string;
  kind: "cpu.sustained_high" | "memory.pressure";
  severity: "warning" | "critical";
  title: string;
  message: string;
  firstSeenAt: number;
  evidence: string[];
  process?: ProcessSample;
}

export interface TimelineEvent {
  id: string;
  occurredAt: number;
  kind: "patrol" | "finding" | "action" | "resolved";
  message: string;
}

export interface CurrentStatus {
  dogState: DogState;
  summary: string;
  healthScore: number;
  snapshot: SystemSnapshot | null;
  findings: Finding[];
  timeline: TimelineEvent[];
  verification: VerificationStatus | null;
}

export interface VerificationStatus {
  targetName: string;
  status: "observing" | "improved" | "noImprovement";
  message: string;
  startedAt: number;
}

export interface MetricPoint {
  capturedAt: number;
  cpuPercent: number;
  memoryPercent: number;
  diskBps: number;
  networkBps: number;
}

export interface HistorySummary {
  points: MetricPoint[];
  baselineCpuPercent: number;
  baselineMemoryPercent: number;
}

export interface LocalDiagnosis {
  summary: string;
  details: string[];
  suggestions: string[];
  confidence: "low" | "medium" | "high";
}

export interface ActionPreview {
  previewId: string;
  action: "terminateProcess";
  riskLevel: "R2" | "R4";
  allowed: boolean;
  title: string;
  warning: string;
  target: ProcessSample;
  expiresAt: number;
}

export interface ActionResult {
  actionId: string;
  success: boolean;
  message: string;
}
