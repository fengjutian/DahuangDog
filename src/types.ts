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
  parentPid: number | null;
  startedAt: number;
  name: string;
  cpuPercent: number;
  memoryBytes: number;
  isCritical: boolean;
}

export interface ApplicationGroup {
  rootPid: number;
  name: string;
  memberCount: number;
  cpuPercent: number;
  memoryBytes: number;
  rootProcess: ProcessSample;
  members: ProcessSample[];
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
  applications: ApplicationGroup[];
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

export interface ProgramRisk {
  pid: number;
  name: string;
  path: string;
  signatureStatus: "valid" | "unverified";
  riskLevel: "normal" | "low" | "medium";
  reasons: string[];
}

export interface StartupEntry {
  name: string;
  command: string;
  source: string;
  riskLevel: "normal" | "medium";
  reasons: string[];
}

export interface SecurityReport {
  scannedAt: number;
  scannedPrograms: number;
  signedPrograms: number;
  programs: ProgramRisk[];
  startupEntries: StartupEntry[];
  summary: string;
}

export interface ActionPreview {
  previewId: string;
  action: "terminateProcess" | "setProcessPriority";
  riskLevel: "R1" | "R2" | "R4";
  allowed: boolean;
  title: string;
  warning: string;
  target: ProcessSample;
  expiresAt: number;
}

export interface UserSettings {
  cpuThreshold: number;
  memoryThreshold: number;
  samplingSeconds: number;
  lowPowerMode: boolean;
  notificationsEnabled: boolean;
  retentionDays: number;
}

export interface ActionResult {
  actionId: string;
  success: boolean;
  message: string;
}
