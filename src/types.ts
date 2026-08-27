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
  threadCount?: number;
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
  diskTotalBytes: number;
  diskAvailableBytes: number;
  uptimeSeconds: number;
  processes: ProcessSample[];
  applications: ApplicationGroup[];
}

export interface Finding {
  id: string;
  kind: string;
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
  peakCpuPercent: number;
  peakMemoryPercent: number;
  averageDiskBps: number;
  averageNetworkBps: number;
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
  securityScore: number;
  mediumRiskCount: number;
  lowRiskCount: number;
  networkConnections: NetworkConnection[];
  scheduledTasks: ScheduledTask[];
  windowsServices: WindowsService[];
}

export interface NetworkConnection { protocol: string; localAddress: string; remoteAddress: string; state: string; pid: number; processName: string; }
export interface ScheduledTask { name: string; path: string; }
export interface WindowsService { name: string; startMode: string; imagePath: string; }

export interface AppUsageAggregate {
  name: string;
  sessionCount: number;
  runtimeSeconds: number;
  foregroundSeconds: number;
  backgroundSeconds: number;
}

export interface AppUsageSummary {
  periodDays: number;
  applicationCount: number;
  sessionCount: number;
  totalRuntimeSeconds: number;
  totalForegroundSeconds: number;
  totalBackgroundSeconds: number;
  longestUsedApp: string | null;
  topApps: AppUsageAggregate[];
  dailyUsage: { date: string; foregroundSeconds: number; launchCount: number }[];
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

export interface AppUsageRecord {
  sessionId: string;
  name: string;
  rootPid: number;
  startedAt: number;
  firstSeenAt: number;
  lastSeenAt: number;
  closedAt: number | null;
  runtimeSeconds: number;
  foregroundSeconds: number;
  backgroundSeconds: number;
  memberPeak: number;
  isRunning: boolean;
}

export interface ActionResult {
  actionId: string;
  success: boolean;
  message: string;
}
