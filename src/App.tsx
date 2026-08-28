import { lazy, memo, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { clearLocalMemory, clearMinimaxApiKey, confirmAction, confirmMaintenance, diagnosePerformance, exportUsageCsv, getAiStatus, getAlerts, getApplicationHistory, getAppUsageHistory, getAppUsageSummary, getCurrentStatus, getHistoryRange, getPeriodicPatterns, getSecurityReport, getSettings, openFileLocation, openProcessLocation, prepareCleanup, preparePriority, prepareStartupChange, prepareTerminate, saveMinimaxApiKey, saveSettings, scanCleanup, testMinimaxConnection, updateAlert } from "./api";
import type { ActionPreview, AlertRecord, ApplicationGroup, ApplicationHistory, AppUsageRecord, AppUsageSummary, CleanupReport, CurrentStatus, HistorySummary, LocalDiagnosis, MetricPoint, PeriodicPattern, ProcessSample, SecurityReport, StartupEntry, SystemSnapshot, UserSettings } from "./types";

const stateLabel: Record<string, string> = {
  idle: "在狗窝待命", patrol: "正在巡逻", suspicious: "竖起耳朵",
  investigating: "正在调查", awaitingApproval: "等你决定",
  verifying: "正在确认效果", resolved: "问题解决"
};

const timelineKindLabel: Record<string, string> = {
  patrol: "巡逻",
  finding: "发现",
  action: "操作",
  resolved: "恢复"
};

const StorageAnalysis = lazy(() => import("./StorageAnalysis"));

type HistoryMetricKey = "cpuPercent" | "memoryPercent" | "diskBps" | "networkBps";
type DogMode = "idle" | "watching" | "scanning" | "thinking" | "success" | "error";

const dogPhrases: Record<DogMode, string[]> = {
  idle: ["这里很安静，我陪你待一会儿。", "大黄在，慢慢来就好。", "摸摸头，我会继续留意电脑。"],
  watching: ["我竖起耳朵了，正在留意这个变化。", "有一点动静，我先帮你看着。"],
  scanning: ["我正在沿着文件夹闻过去，很快告诉你。", "已经记住走过的地方，中断也能接着找。"],
  thinking: ["让我认真想一会儿，先不用着急。", "我在把线索排好，很快回来。"],
  success: ["处理好了！今天也守住啦。", "完成啦，摇摇尾巴庆祝一下。"],
  error: ["这里暂时过不去，休息一下再试也可以。", "没有弄坏任何东西，我把原因记下来了。"],
};

function DogCompanion({ mode, personality, quiet, reducedMotion, onDiagnose }: { mode: DogMode; personality: UserSettings["companionPersonality"]; quiet: boolean; reducedMotion: boolean; onDiagnose: () => void }) {
  const [bubble, setBubble] = useState("");
  const [idleTrick, setIdleTrick] = useState<"" | "look" | "stretch">("");
  const [petting, setPetting] = useState(false);
  const phraseIndex = useRef(0);
  const speak = useCallback(() => {
    const phrases = dogPhrases[mode];
    phraseIndex.current = (phraseIndex.current + 1) % phrases.length;
    setBubble(phrases[phraseIndex.current]);
    setPetting(false);
    window.requestAnimationFrame(() => setPetting(true));
    window.setTimeout(() => setPetting(false), 900);
  }, [mode]);
  useEffect(() => {
    if (quiet || personality === "quiet") return;
    const phrases = dogPhrases[mode];
    setBubble(phrases[personality === "playful" ? Math.floor(Math.random() * phrases.length) : 0]);
    const timer = window.setTimeout(() => setBubble(""), personality === "playful" ? 4800 : 3500);
    return () => window.clearTimeout(timer);
  }, [mode, personality, quiet]);
  useEffect(() => {
    if (mode !== "idle" || reducedMotion || quiet || personality === "quiet" || document.hidden) { setIdleTrick(""); return; }
    const delay = personality === "playful" ? 5500 : 9500;
    const timer = window.setInterval(() => {
      setIdleTrick(current => current === "look" ? "stretch" : "look");
      window.setTimeout(() => setIdleTrick(""), 1400);
    }, delay);
    return () => window.clearInterval(timer);
  }, [mode, personality, quiet, reducedMotion]);
  return <div className={`dog-companion dog-${mode} ${idleTrick ? `dog-${idleTrick}` : ""} ${petting ? "dog-petting" : ""} ${reducedMotion ? "reduced" : ""}`}>
    {bubble && <div className="dog-bubble" role="status">{bubble}</div>}
    <button className="avatar" onClick={speak} onDoubleClick={onDiagnose} aria-label="和大黄互动，双击开始诊断" title="点击摸摸头 · 双击立即诊断"><span className="dog-face" aria-hidden="true">🐕</span><i aria-hidden="true"/><em className="dog-sparkles" aria-hidden="true"><b>♥</b><b>✦</b><b>🐾</b></em></button>
  </div>;
}

function VirtualList<T>({ items, itemHeight, height = 430, className = "", keyFor, renderItem }: { items: T[]; itemHeight: number; height?: number; className?: string; keyFor: (item: T, index: number) => string; renderItem: (item: T, index: number) => React.ReactNode }) {
  const [scrollTop, setScrollTop] = useState(0);
  const host = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const maximum = Math.max(0, items.length * itemHeight - height);
    if (scrollTop > maximum) { setScrollTop(maximum); if (host.current) host.current.scrollTop = maximum; }
  }, [height, itemHeight, items.length, scrollTop]);
  const overscan = 5;
  const start = Math.max(0, Math.floor(scrollTop / itemHeight) - overscan);
  const end = Math.min(items.length, Math.ceil((scrollTop + height) / itemHeight) + overscan);
  return <div ref={host} className={`virtual-list ${className}`} style={{ height }} onScroll={event => setScrollTop(event.currentTarget.scrollTop)}>
    <div className="virtual-list-spacer" style={{ height: items.length * itemHeight }}>
      {items.slice(start, end).map((item, offset) => { const index = start + offset; return <div className="virtual-list-item" style={{ height: itemHeight, transform: `translateY(${index * itemHeight}px)` }} key={keyFor(item, index)}>{renderItem(item, index)}</div>; })}
    </div>
  </div>;
}

const historyMetricLabel: Record<HistoryMetricKey, string> = {
  cpuPercent: "CPU",
  memoryPercent: "内存",
  diskBps: "磁盘读写",
  networkBps: "网络收发"
};

function formatBytes(value: number): string {
  if (!Number.isFinite(value)) return "--";
  const gb = value / 1024 / 1024 / 1024;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(value / 1024 / 1024).toFixed(0)} MB`;
}

function formatRate(value: number): string {
  if (!Number.isFinite(value)) return "--";
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB/s`;
  if (value >= 1024) return `${(value / 1024).toFixed(0)} KB/s`;
  return `${value.toFixed(0)} B/s`;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds} 秒`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟`;
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor(seconds % 3600 / 60);
  return `${hours} 小时 ${minutes} 分钟`;
}

const fullDateTimeFormatter = new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" });
const clockFormatter = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const compactDateTimeFormatter = new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });

function formatFullDateTime(timestamp: number): string { return fullDateTimeFormatter.format(new Date(timestamp)); }
function formatClock(timestamp: number): string { return clockFormatter.format(new Date(timestamp)); }

const TimelineCard = memo(function TimelineCard({ items, onOpen }: { items: CurrentStatus["timeline"]; onOpen: () => void }) {
  return <section className="card"><div className="section-title"><h3>🐾 巡逻记录</h3><div className="timeline-title-actions"><span>最近事件</span>{items.length > 8 && <button onClick={onOpen}>查看全部 {items.length} 条</button>}</div></div>
    <ol className="timeline">{items.slice(0, 8).map(item => <li key={item.id} className={`timeline-${item.kind}`}><time>{formatClock(item.occurredAt)}</time><span>{item.message}</span></li>)}</ol>
  </section>;
});

const knownApplications: Record<string, { productName: string; description: string }> = {
  "msedge.exe": { productName: "Microsoft Edge", description: "微软的 Chromium 浏览器，用于网页浏览、扩展和 Web 应用。多个进程分别负责标签页、GPU、网络与扩展。" },
  "msedgewebview2.exe": { productName: "Microsoft Edge WebView2 Runtime", description: "为 Windows 桌面应用提供网页界面渲染能力，通常由其他应用在后台启动。" },
  "code.exe": { productName: "Visual Studio Code", description: "微软开发的代码编辑器。多个进程通常分别负责窗口、扩展、终端和语言服务。" },
  "feishu.exe": { productName: "飞书", description: "团队沟通与办公协作应用，包含消息、会议、文档和云端协作功能。" },
  "wechatappex.exe": { productName: "微信辅助进程", description: "微信桌面版使用的应用与内容辅助进程，可能负责小程序、网页内容或独立窗口。" },
  "weixin.exe": { productName: "微信", description: "腾讯微信桌面客户端，用于消息、通话、文件传输和小程序。" },
  "termius.exe": { productName: "Termius", description: "SSH 与远程服务器管理客户端，用于终端连接、主机管理和文件传输。" },
  "podman desktop.exe": { productName: "Podman Desktop", description: "容器与 Kubernetes 桌面管理工具，用于管理本地容器、镜像和开发环境。" },
  "minimax code.exe": { productName: "MiniMax Code", description: "面向软件开发的代码工具与智能编程客户端。" }
};

function applicationPresentation(application: ApplicationGroup) {
  const known = knownApplications[application.name.toLowerCase()];
  const productName = application.productName?.trim() || known?.productName || application.name;
  const fileDescription = application.description?.trim();
  const description = fileDescription && ![application.name, productName].some(value => value.toLowerCase() === fileDescription.toLowerCase())
    ? fileDescription
    : known?.description || `这是由 ${application.name} 启动的 Windows 应用。程序文件没有提供更具体的用途说明。`;
  return { productName, description };
}

const Metric = memo(function Metric({ label, value, tone, onClick }: { label: string; value: string; tone?: "warn"; onClick?: () => void }) {
  if (onClick) return <button className={`metric metric-clickable ${tone ?? ""}`} onClick={onClick} aria-label={`查看${label}历史明细`}><span>{label}</span><strong>{value}</strong><small>查看历史</small></button>;
  return <div className={`metric ${tone ?? ""}`}><span>{label}</span><strong>{value}</strong></div>;
});

const DashboardMetrics = memo(function DashboardMetrics({ snap, findingCount, onHistory, onStorage }: { snap: SystemSnapshot | null; findingCount: number; onHistory: (metric: HistoryMetricKey) => void; onStorage: () => void }) {
  return <section className="metrics">
    <Metric label="CPU" value={snap ? `${snap.cpuPercent.toFixed(0)}%` : "--"} tone={snap && snap.cpuPercent >= 90 ? "warn" : undefined} onClick={() => onHistory("cpuPercent")} />
    <Metric label="内存" value={snap ? `${snap.memoryPercent.toFixed(0)}%` : "--"} tone={snap && snap.memoryPercent >= 90 ? "warn" : undefined} onClick={() => onHistory("memoryPercent")} />
    <Metric label="已用内存" value={snap ? formatBytes(snap.usedMemoryBytes) : "--"} />
    <Metric label="磁盘读 / 写" value={snap ? `${formatRate(snap.diskReadBps)} / ${formatRate(snap.diskWriteBps)}` : "--"} onClick={() => onHistory("diskBps")} />
    <Metric label="网络下 / 上" value={snap ? `${formatRate(snap.networkReceiveBps)} / ${formatRate(snap.networkSendBps)}` : "--"} onClick={() => onHistory("networkBps")} />
    <Metric label="发现" value={`${findingCount} 个`} tone={findingCount ? "warn" : undefined} />
    <Metric label="磁盘空间" value={snap ? `${formatBytes(snap.diskAvailableBytes)} 可用` : "--"} tone={snap && snap.diskTotalBytes > 0 && snap.diskAvailableBytes / snap.diskTotalBytes < .1 ? "warn" : undefined} onClick={onStorage} />
    <Metric label="系统运行" value={snap ? formatDuration(snap.uptimeSeconds) : "--"} />
    <Metric label="进程 / 线程" value={snap ? `${snap.processes.length}+ / ${snap.processes.reduce((sum, process) => sum + (process.threadCount ?? 0), 0)}+` : "--"} />
  </section>;
});

function linePath(points: MetricPoint[], key: "cpuPercent" | "memoryPercent" | "diskBps" | "networkBps"): string {
  if (points.length < 2) return "";
  const max = key === "cpuPercent" || key === "memoryPercent" ? 100 : Math.max(...points.map(item => item[key]), 1);
  return points.map((point, index) => {
    const x = index / (points.length - 1) * 100;
    const y = 100 - Math.max(0, Math.min(100, point[key] / max * 100));
    return `${index ? "L" : "M"}${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
}

type SystemMetricKey = "cpuPercent" | "memoryPercent" | "diskBps" | "networkBps";

function formatSystemAxisValue(key: SystemMetricKey, value: number): string {
  return key === "cpuPercent" || key === "memoryPercent" ? `${value.toFixed(0)}%` : formatRate(value);
}

function SystemHistoryChart({ history, range, metric, label, style }: { history: HistorySummary; range: number; metric: SystemMetricKey; label: string; style: string }) {
  const peak = metric === "cpuPercent" || metric === "memoryPercent" ? 100 : Math.max(...history.points.map(point => point[metric]), 1);
  const first = history.points.at(0)?.capturedAt;
  const middle = history.points.at(Math.floor((history.points.length - 1) / 2))?.capturedAt;
  const last = history.points.at(-1)?.capturedAt;
  return <div className="mini-trend">
    <b>{label}</b>
    <div className="app-chart-layout system-chart-layout">
      <div className="app-chart-y-axis"><span>{formatSystemAxisValue(metric, peak)}</span><span>{formatSystemAxisValue(metric, peak / 2)}</span><span>{formatSystemAxisValue(metric, 0)}</span></div>
      <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-label={`${label}资源趋势`}><path className="app-chart-grid" d="M0,0 H100 M0,50 H100 M0,100 H100 M0,0 V100 M50,0 V100 M100,0 V100"/><path className={style} d={linePath(history.points, metric)}/></svg>
      <div className="app-chart-x-axis"><time>{first == null ? "--" : formatApplicationAxisTime(first, range)}</time><time>{middle == null ? "--" : formatApplicationAxisTime(middle, range)}</time><time>{last == null ? "--" : formatApplicationAxisTime(last, range)}</time></div>
    </div>
  </div>;
}

function DailyUsageChart({ days }: { days: AppUsageSummary["dailyUsage"] }) {
  const peak = Math.max(...days.map(item => item.foregroundSeconds), 1);
  return <div className="daily-usage-chart">
    <div className="daily-usage-y-axis"><span>{formatDuration(peak)}</span><span>{formatDuration(Math.round(peak / 2))}</span><span>0</span></div>
    <div className="daily-usage-bars">{days.map(day => <span key={day.date} title={`${day.date} · 前台 ${formatDuration(day.foregroundSeconds)} · 启动 ${day.launchCount} 次`}><i style={{height: `${Math.max(5, day.foregroundSeconds / peak * 100)}%`}}/><small>{day.date.slice(5)}</small><b>{day.launchCount} 次</b></span>)}</div>
    <div className="daily-axis-title"><span>Y：前台使用时长</span><span>X：日期</span></div>
  </div>;
}

type ApplicationMetricKey = "cpuPercent" | "memoryBytes" | "diskReadBps" | "diskWriteBps";

function appLinePath(history: ApplicationHistory, key: ApplicationMetricKey): string {
  if (history.points.length < 2) return "";
  const peak = Math.max(...history.points.map(point => point[key]), 1);
  return history.points.map((point, index) => `${index ? "L" : "M"}${(index / (history.points.length - 1) * 100).toFixed(2)},${(100 - point[key] / peak * 100).toFixed(2)}`).join(" ");
}

function formatApplicationAxisValue(key: ApplicationMetricKey, value: number): string {
  if (key === "cpuPercent") return `${value.toFixed(value >= 10 ? 0 : 1)}%`;
  if (key === "memoryBytes") return formatBytes(value);
  return formatRate(value);
}

function formatApplicationAxisTime(timestamp: number, rangeMinutes: number): string {
  const date = new Date(timestamp);
  return rangeMinutes <= 60
    ? clockFormatter.format(date)
    : compactDateTimeFormatter.format(date);
}

function ApplicationHistoryChart({ history, metric, label }: { history: ApplicationHistory; metric: ApplicationMetricKey; label: string }) {
  const peak = Math.max(...history.points.map(point => point[metric]), 1);
  const first = history.points.at(0)?.capturedAt;
  const middle = history.points.at(Math.floor((history.points.length - 1) / 2))?.capturedAt;
  const last = history.points.at(-1)?.capturedAt;
  return <article>
    <b>{label}</b>
    <div className="app-chart-layout">
      <div className="app-chart-y-axis"><span>{formatApplicationAxisValue(metric, peak)}</span><span>{formatApplicationAxisValue(metric, peak / 2)}</span><span>{formatApplicationAxisValue(metric, 0)}</span></div>
      <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-label={`${label} 历史曲线`}>
        <path className="app-chart-grid" d="M0,0 H100 M0,50 H100 M0,100 H100 M0,0 V100 M50,0 V100 M100,0 V100" />
        <path className="app-chart-data" d={appLinePath(history, metric)} />
      </svg>
      <div className="app-chart-x-axis"><time>{first == null ? "--" : formatApplicationAxisTime(first, history.rangeMinutes)}</time><time>{middle == null ? "--" : formatApplicationAxisTime(middle, history.rangeMinutes)}</time><time>{last == null ? "--" : formatApplicationAxisTime(last, history.rangeMinutes)}</time></div>
    </div>
    <small>{history.points.length} 个采样点 · Y 轴峰值 {formatApplicationAxisValue(metric, peak)}</small>
  </article>;
}

const TrendChart = memo(function TrendChart({ history, range, onRange }: { history: HistorySummary; range: number; onRange: (range: number) => void }) {
  return <section className="card trend-card">
    <div className="section-title"><h3>资源趋势</h3><div className="range-tabs">{[[10,"10 分钟"],[60,"1 小时"],[1440,"24 小时"],[10080,"7 天"]].map(([value,label]) => <button key={value} className={range === value ? "active" : ""} onClick={() => onRange(Number(value))}>{label}</button>)}</div></div>
    <div className="trend-grid">{([['cpuPercent','CPU','cpu-line'],['memoryPercent','内存','memory-line'],['diskBps','磁盘吞吐','disk-line'],['networkBps','网络吞吐','network-line']] as const).map(([key,label,style]) => <SystemHistoryChart key={key} history={history} range={range} metric={key} label={label} style={style} />)}</div>
    <div className="monitor-insights">
      <div><span>CPU 峰值</span><b>{history.peakCpuPercent.toFixed(0)}%</b></div>
      <div><span>内存峰值</span><b>{history.peakMemoryPercent.toFixed(0)}%</b></div>
      <div><span>平均磁盘</span><b>{formatRate(history.averageDiskBps)}</b></div>
      <div><span>平均网络</span><b>{formatRate(history.averageNetworkBps)}</b></div>
    </div>
  </section>;
});

export default function App() {
  const pageVisible = useRef(!document.hidden);
  const [status, setStatus] = useState<CurrentStatus | null>(null);
  const [preview, setPreview] = useState<ActionPreview | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<HistorySummary | null>(null);
  const [historyError, setHistoryError] = useState("");
  const [historyRange, setHistoryRange] = useState(10);
  const [diagnosis, setDiagnosis] = useState<LocalDiagnosis | null>(null);
  const [diagnosisOpen, setDiagnosisOpen] = useState(false);
  const [diagnosisLoading, setDiagnosisLoading] = useState(false);
  const [security, setSecurity] = useState<SecurityReport | null>(null);
  const [selected, setSelected] = useState<ProcessSample | null>(null);
  const [settings, setSettings] = useState<UserSettings | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<"patrol" | "companion" | "system" | "ai" | "data">("patrol");
  const [minimaxKey, setMinimaxKey] = useState("");
  const [aiConfigured, setAiConfigured] = useState(false);
  const [aiTesting, setAiTesting] = useState(false);
  const [aiTestResult, setAiTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [expandedApp, setExpandedApp] = useState<number | null>(null);
  const [selectedApp, setSelectedApp] = useState<number | null>(null);
  const appDetailScrollRef = useRef<HTMLDivElement | null>(null);
  const appDetailScrollTop = useRef(0);
  const [usage, setUsage] = useState<AppUsageRecord[] | null>(null);
  const [usageSummary, setUsageSummary] = useState<AppUsageSummary | null>(null);
  const [usageQuery, setUsageQuery] = useState("");
  const [usageTab, setUsageTab] = useState<"charts" | "list">("charts");
  const [usagePeriod, setUsagePeriod] = useState(7);
  const [securityFilter, setSecurityFilter] = useState<"all" | "medium" | "low">("all");
  const [securityTab, setSecurityTab] = useState<"overview" | "programs" | "startup" | "network" | "tasks" | "services">("overview");
  const [hardwareOpen, setHardwareOpen] = useState(false);
  const [storageOpen, setStorageOpen] = useState(false);
  const [hardwareTab, setHardwareTab] = useState<"cpu" | "gpu" | "power" | "disks" | "network" | "apps">("cpu");
  const [processQuery, setProcessQuery] = useState("");
  const [timelineOpen, setTimelineOpen] = useState(false);
  const [selectedHistoryMetric, setSelectedHistoryMetric] = useState<HistoryMetricKey | null>(null);
  const [appHistory, setAppHistory] = useState<ApplicationHistory | null>(null);
  const [appHistoryRange, setAppHistoryRange] = useState(60);
  const [appHistoryReturnApp, setAppHistoryReturnApp] = useState<number | null>(null);
  const [alerts, setAlerts] = useState<AlertRecord[] | null>(null);
  const [alertFilter, setAlertFilter] = useState("");
  const [patterns, setPatterns] = useState<PeriodicPattern[] | null>(null);
  const [cleanup, setCleanup] = useState<CleanupReport | null>(null);
  const [cleanupSelection, setCleanupSelection] = useState<Set<string>>(() => new Set());
  const [cleanupFilter, setCleanupFilter] = useState("all");
  const [dogActivity, setDogActivity] = useState<DogMode>("idle");
  const closeDiagnosis = useCallback(() => setDiagnosisOpen(false), []);

  const refresh = useCallback(async () => {
    try { setStatus(await getCurrentStatus()); }
    catch (error) { setMessage(`暂时没听见系统消息：${String(error)}`); }
  }, []);

  useEffect(() => {
    void refresh();
    const tauriRuntime = "__TAURI_INTERNALS__" in window;
    const timer = tauriRuntime ? undefined : window.setInterval(() => { if (pageVisible.current) void refresh(); }, 3000);
    let unlisten: undefined | (() => void);
    if (tauriRuntime) {
      void listen<CurrentStatus>("status://updated", event => { if (pageVisible.current) setStatus(event.payload); }).then(fn => { unlisten = fn; });
    }
    return () => { if (timer != null) window.clearInterval(timer); unlisten?.(); };
  }, [refresh]);

  useEffect(() => {
    const onVisibilityChange = () => {
      pageVisible.current = !document.hidden;
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => document.removeEventListener("visibilitychange", onVisibilityChange);
  }, [refresh]);

  useEffect(() => {
    const load = () => { if (pageVisible.current) void getHistoryRange(historyRange).then(result => { setHistory(result); setHistoryError(""); }).catch(error => setHistoryError(String(error))); };
    load();
    const timer = window.setInterval(load, 10_000);
    return () => window.clearInterval(timer);
  }, [historyRange]);

  useEffect(() => { void Promise.all([getSettings(), getAiStatus()]).then(([nextSettings, ai]) => { setSettings(nextSettings); setAiConfigured(ai.configured); }).catch(() => undefined); }, []);

  useEffect(() => {
    if (!message) return;
    const failed = /失败|错误|无法|异常|拒绝/.test(message);
    setDogActivity(failed ? "error" : "success");
    const timer = window.setTimeout(() => setDogActivity("idle"), 3500);
    return () => window.clearTimeout(timer);
  }, [message]);

  useEffect(() => {
    if (!diagnosisOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === "Escape") closeDiagnosis(); };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [closeDiagnosis, diagnosisOpen]);

  useLayoutEffect(() => {
    if (selectedApp != null && appDetailScrollRef.current) {
      appDetailScrollRef.current.scrollTop = appDetailScrollTop.current;
    }
  }, [selectedApp, status?.snapshot?.capturedAt]);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let unlisten: undefined | (() => void);
    void listen<string>("ui://open-panel", event => {
      if (event.payload === "hardware") setHardwareOpen(true);
      if (event.payload === "usage") void showUsage();
      if (event.payload === "security") void scanSecurity();
    }).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, []);

  async function inspect(process: ProcessSample) {
    setSelected(process);
  }

  async function showApplicationHistory(name: string, range = appHistoryRange) {
    setBusy(true);
    try { setAppHistoryRange(range); setAppHistory(await getApplicationHistory(name, range)); setSelected(null); setSelectedApp(null); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  function closeApplicationHistory() {
    setAppHistory(null);
    if (appHistoryReturnApp != null) {
      setSelectedApp(appHistoryReturnApp);
      setAppHistoryReturnApp(null);
    }
  }

  async function showAlerts(filter = alertFilter) {
    setBusy(true);
    try { setAlertFilter(filter); setAlerts(await getAlerts(filter)); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  async function changeAlert(alert: AlertRecord, next: AlertRecord["status"]) {
    const note = window.prompt("处理备注（可留空）", alert.note) ?? alert.note;
    try { await updateAlert(alert.id, next, note); await showAlerts(); }
    catch (error) { setMessage(String(error)); }
  }

  async function showPatterns() {
    setBusy(true);
    try { setPatterns(await getPeriodicPatterns(30)); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  async function showCleanup() {
    setBusy(true);
    try { const report = await scanCleanup(); setCleanup(report); setCleanupFilter("all"); setCleanupSelection(new Set(report.candidates.filter(item => item.cleanable).slice(0, 1_000).map(item => item.path))); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  async function cleanSelected() {
    try { const preview = await prepareCleanup([...cleanupSelection]); if (!window.confirm(`${preview.title}\n${preview.itemCount} 个文件，共 ${formatBytes(preview.totalBytes)}\n\n${preview.warning}`)) return; const result = await confirmMaintenance(preview.previewId); setMessage(result.message); await showCleanup(); }
    catch (error) { setMessage(String(error)); }
  }

  async function changeStartup(entry: StartupEntry) {
    const enable = entry.source === "大黄狗\\已禁用";
    try { const preview = await prepareStartupChange(entry.source, entry.name, entry.command, enable); if (!window.confirm(`${preview.title}\n\n${preview.warning}`)) return; const result = await confirmMaintenance(preview.previewId); setMessage(result.message); setSecurity(await getSecurityReport()); }
    catch (error) { setMessage(String(error)); }
  }

  async function requestTerminate(process: ProcessSample) {
    try { setPreview(await prepareTerminate(process.pid, process.startedAt)); setSelected(null); }
    catch (error) { setMessage(String(error)); }
  }

  async function requestPriority(process: ProcessSample, level: "belowNormal" | "normal" | "aboveNormal") {
    try { setPreview(await preparePriority(process.pid, process.startedAt, level)); setSelected(null); }
    catch (error) { setMessage(String(error)); }
  }

  async function reveal(process: ProcessSample) {
    try { const result = await openProcessLocation(process.pid, process.startedAt); setMessage(result.message); setSelected(null); }
    catch (error) { setMessage(String(error)); }
  }

  async function execute() {
    if (!preview?.allowed) return;
    setBusy(true);
    try {
      const result = await confirmAction(preview.previewId);
      setMessage(result.message);
      setPreview(null);
      await refresh();
    } catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  async function diagnose() {
    setBusy(true); setDiagnosisLoading(true); setDiagnosisOpen(true); setDiagnosis(null); setDogActivity("thinking");
    try { setDiagnosis(await diagnosePerformance()); setDogActivity("success"); }
    catch (error) { setMessage(String(error)); setDiagnosisOpen(false); setDogActivity("error"); }
    finally { setBusy(false); setDiagnosisLoading(false); window.setTimeout(() => setDogActivity("idle"), 4000); }
  }

  async function testMinimax() {
    if (!settings) return;
    if (!minimaxKey.trim() && !aiConfigured) { setAiTestResult({ ok: false, message: "请先输入 API Key" }); return; }
    setAiTesting(true); setAiTestResult(null);
    try { setAiTestResult({ ok: true, message: await testMinimaxConnection(settings.minimaxModel, minimaxKey) }); }
    catch (error) { setAiTestResult({ ok: false, message: String(error) }); }
    finally { setAiTesting(false); }
  }

  async function scanSecurity() {
    setBusy(true);
    try { setSecurity(await getSecurityReport()); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  async function revealSecurityFile(path: string) {
    try { const result = await openFileLocation(path); setMessage(result.message); }
    catch (error) { setMessage(String(error)); }
  }

  async function persistSettings() {
    if (!settings) return;
    setBusy(true);
    try {
      let configured = aiConfigured;
      if (minimaxKey.trim()) {
        const ai = await saveMinimaxApiKey(minimaxKey);
        configured = ai.configured;
        setAiConfigured(ai.configured);
        setMinimaxKey("");
      }
      if (settings.minimaxEnabled && !configured) throw new Error("启用 MiniMax 前请先输入 API Key");
      setSettings(await saveSettings(settings)); setMessage("巡逻设置已经保存。" ); closeSettings();
    }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  function closeSettings() {
    setSettingsOpen(false);
    setMinimaxKey("");
    setAiTestResult(null);
  }

  async function removeMinimaxKey() {
    if (!window.confirm("确定从 Windows 凭据管理器删除 MiniMax API Key 吗？")) return;
    try { const ai = await clearMinimaxApiKey(); setAiConfigured(ai.configured); setMinimaxKey(""); setMessage("MiniMax API Key 已删除。" ); }
    catch (error) { setMessage(String(error)); }
  }

  async function clearMemory() {
    if (!window.confirm("确定清除所有历史快照、巡逻记录和动作审计吗？这个操作不能撤销。")) return;
    try { await clearLocalMemory(); setHistory(null); setMessage("旧的本地记忆已经清除。" ); await refresh(); }
    catch (error) { setMessage(String(error)); }
  }

  async function showUsage() {
    setBusy(true);
    try {
      const [records, summary] = await Promise.all([getAppUsageHistory(), getAppUsageSummary(usagePeriod)]);
      setUsage(records);
      setUsageSummary(summary);
    }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  async function changeUsagePeriod(period: number) {
    setUsagePeriod(period);
    setBusy(true);
    try { setUsageSummary(await getAppUsageSummary(period)); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  async function exportUsage() {
    const rows = visibleUsage.map(record => [record.name, record.rootPid, new Date(record.startedAt).toLocaleString("zh-CN"), record.closedAt ? new Date(record.closedAt).toLocaleString("zh-CN") : "运行中", record.runtimeSeconds, record.foregroundSeconds, record.backgroundSeconds, record.memberPeak]);
    const escape = (value: string | number) => `"${String(value).replaceAll('"', '""')}"`;
    const csv = [["应用", "PID", "启动时间", "关闭时间", "运行秒数", "前台秒数", "后台秒数", "进程峰值"], ...rows].map(row => row.map(escape).join(",")).join("\r\n");
    try { const result = await exportUsageCsv(csv); setMessage(result.message); }
    catch (error) { setMessage(String(error)); }
  }

  const snap = status?.snapshot ?? null;
  const visibleUsage = useMemo(() => {
    const usageSince = Date.now() - usagePeriod * 24 * 60 * 60 * 1000;
    const query = usageQuery.trim().toLowerCase();
    return usage?.filter(record => record.lastSeenAt >= usageSince && record.name.toLowerCase().includes(query)) ?? [];
  }, [usage, usagePeriod, usageQuery]);
  const visiblePrograms = useMemo(() => security?.programs.filter(program => securityFilter === "all" || program.riskLevel === securityFilter) ?? [], [security, securityFilter]);
  const normalizedProcessQuery = processQuery.trim().toLowerCase();
  const visibleApplications = useMemo(() => snap?.applications.filter(application => {
    if (!normalizedProcessQuery) return true;
    return application.name.toLowerCase().includes(normalizedProcessQuery)
      || String(application.rootPid).includes(normalizedProcessQuery)
      || application.members.some(process => process.name.toLowerCase().includes(normalizedProcessQuery)
        || String(process.pid).includes(normalizedProcessQuery));
  }).slice(0, 8) ?? [], [snap?.applications, normalizedProcessQuery]);
  const appDetails = selectedApp == null ? null : snap?.applications.find(application => application.rootPid === selectedApp) ?? null;
  const appDetailsPresentation = appDetails ? applicationPresentation(appDetails) : null;
  const cleanupCategories = useMemo(() => cleanup ? [...new Set(cleanup.candidates.map(item => item.category))] : [], [cleanup]);
  const visibleCleanup = useMemo(() => cleanup?.candidates.filter(item => cleanupFilter === "all" || item.category === cleanupFilter) ?? [], [cleanup, cleanupFilter]);
  const reversedHistoryPoints = useMemo(() => history ? [...history.points].reverse() : [], [history]);
  const openHistory = useCallback((metric: HistoryMetricKey) => setSelectedHistoryMetric(metric), []);
  const openStorage = useCallback(() => setStorageOpen(true), []);
  const openTimeline = useCallback(() => setTimelineOpen(true), []);

  if (!status) return <main className="loading">🐕 大黄狗正在醒来……</main>;

  const dogMode: DogMode = diagnosisLoading ? "thinking" : dogActivity !== "idle" ? dogActivity : status.findings.length ? "watching" : "idle";
  const companion = settings ?? { companionPersonality: "warm" as const, companionQuietMode: false, reduceCompanionMotion: false };
  return <main className="shell" data-reduce-motion={companion.reduceCompanionMotion || undefined}>
    <header><div className="brand"><span className="dog">🐕</span><div><h1>大黄狗</h1><p>住在 Windows 里的 AI 看门狗</p></div></div><div className="header-actions"><button onClick={() => setStorageOpen(true)}>💾 存储分析</button><button onClick={() => setHardwareOpen(true)}>🖥️ 硬件</button><button onClick={showUsage} disabled={busy}>⏱ 使用记录</button><button onClick={() => void showAlerts()} disabled={busy}>🔔 告警</button><button onClick={showPatterns} disabled={busy}>🕒 规律</button><button onClick={showCleanup} disabled={busy}>🧹 清理</button><button onClick={scanSecurity} disabled={busy}>🛡️ 看门报告</button></div></header>

    <section className="hero">
      <DogCompanion mode={dogMode} personality={companion.companionPersonality} quiet={companion.companionQuietMode} reducedMotion={companion.reduceCompanionMotion} onDiagnose={() => void diagnose()} />
      <div><span className="eyebrow">今天的巡逻报告</span><h2>{status.summary}</h2><p>健康度 <b>{status.healthScore}</b> / 100</p><button className="diagnose-button" onClick={diagnose} disabled={busy}>{busy ? "正在检查…" : "大黄，电脑为什么卡？"}</button></div>
    </section>

    <DashboardMetrics snap={snap} findingCount={status.findings.length} onHistory={openHistory} onStorage={openStorage} />

    {status.verification && <section className={`verification ${status.verification.status}`}>
      <span>{status.verification.status === "observing" ? "👀" : status.verification.status === "improved" ? "✅" : "🤔"}</span>
      <div><b>{status.verification.targetName}</b><p>{status.verification.message}</p></div>
    </section>}

    {status.findings.length > 0 && <section className="findings">
      {status.findings.map(finding => <article key={finding.id} className="finding-card">
        <div><span className="risk">{finding.severity === "critical" ? "严重" : "需要注意"}</span><h3>{finding.title}</h3><p>{finding.message}</p></div>
        <ul>{finding.evidence.map(item => <li key={item}>{item}</li>)}</ul>
        {finding.process && <button onClick={() => inspect(finding.process!)}>查看并处理 {finding.process.name}</button>}
      </article>)}
    </section>}

    {history && <TrendChart history={history} range={historyRange} onRange={setHistoryRange} />}

    <div className="columns">
      <section className="card"><div className="section-title"><h3>正在盯着</h3><span>应用总占用 · 点击展开子进程</span></div>
        <div className="process-search-wrap"><span aria-hidden="true">⌕</span><input className="process-search" value={processQuery} onChange={event => setProcessQuery(event.target.value)} placeholder="搜索应用、进程或 PID" aria-label="搜索应用、进程或 PID" />{processQuery && <button onClick={() => setProcessQuery("")} aria-label="清除进程搜索">×</button>}</div>
        <div className="process-list">{visibleApplications.map(app => { const presentation = applicationPresentation(app); return <div className="app-group" key={`${app.rootPid}-${app.name}`}>
          <div className="process application app-summary-row">
            <button className="process-expand" onClick={() => setExpandedApp(expandedApp === app.rootPid ? null : app.rootPid)} aria-label={expandedApp === app.rootPid ? `收起 ${app.name} 进程` : `展开 ${app.name} 进程`}>{app.rootProcess.isCritical ? "🛡️" : expandedApp === app.rootPid ? "▾" : "▸"}</button>
            <button className="app-detail-trigger" onClick={() => { appDetailScrollTop.current = 0; setSelectedApp(app.rootPid); }}>
              <span className="process-name"><b>{presentation.productName}</b><small>{app.name} · {app.memberCount} 个进程 · 主 PID {app.rootPid}</small></span>
              <span className="process-summary"><b>{app.cpuPercent.toFixed(1)}%</b><small>{formatBytes(app.memoryBytes)}</small></span>
            </button>
          </div>
          {expandedApp === app.rootPid && <div className="child-processes">{app.members.map(process => <button className="process child" key={`${process.pid}-${process.startedAt}`} onClick={() => inspect(process)}>
            <span className="process-icon">└</span><span className="process-name"><b>{process.pid === app.rootPid ? "主进程" : "子进程"}</b><small>PID {process.pid}{process.parentPid ? ` · 父 PID ${process.parentPid}` : ""} · {process.threadCount ?? 0} 线程</small></span><span><b>{process.cpuPercent.toFixed(1)}%</b><small>{formatBytes(process.memoryBytes)}</small></span>
          </button>)}</div>}
        </div>})}{!visibleApplications.length && <p className="empty">{normalizedProcessQuery ? "没有找到匹配的应用或进程。" : "还没有采集到应用数据。"}</p>}</div>
      </section>

      <TimelineCard items={status.timeline} onOpen={openTimeline} />
    </div>

    {message && <div className="toast" onClick={() => setMessage("")}>{message}</div>}
    {selectedHistoryMetric && <div className="modal-backdrop" onClick={() => setSelectedHistoryMetric(null)}><section className="modal report-modal metric-history-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">每个采样时间点</span><h3>{historyMetricLabel[selectedHistoryMetric]}历史明细</h3></div><button className="modal-close" onClick={() => setSelectedHistoryMetric(null)} aria-label="关闭历史明细">×</button></div>
      <div className="range-tabs metric-history-ranges">{[[10,"10 分钟"],[60,"1 小时"],[1440,"24 小时"],[10080,"7 天"]].map(([value,label]) => <button key={value} className={historyRange === value ? "active" : ""} onClick={() => setHistoryRange(Number(value))}>{label}</button>)}</div>
      <div className="metric-history-scroll">{historyError ? <p className="history-error">历史数据读取失败：{historyError}</p> : history?.points.length ? <div className="metric-history-table" role="table">
        <div className="metric-history-row metric-history-head" role="row"><b>采样时间</b>{(["cpuPercent","memoryPercent","diskBps","networkBps"] as HistoryMetricKey[]).map(key => <b key={key} className={selectedHistoryMetric === key ? "selected" : ""}>{historyMetricLabel[key]}</b>)}</div>
        <VirtualList items={reversedHistoryPoints} itemHeight={40} className="metric-history-virtual" keyFor={point => String(point.capturedAt)} renderItem={point => <div className="metric-history-row" role="row"><time>{formatFullDateTime(point.capturedAt)}</time><span className={selectedHistoryMetric === "cpuPercent" ? "selected" : ""}>{point.cpuPercent.toFixed(1)}%</span><span className={selectedHistoryMetric === "memoryPercent" ? "selected" : ""}>{point.memoryPercent.toFixed(1)}%</span><span className={selectedHistoryMetric === "diskBps" ? "selected" : ""}>{formatRate(point.diskBps)}</span><span className={selectedHistoryMetric === "networkBps" ? "selected" : ""}>{formatRate(point.networkBps)}</span></div>} />
      </div> : <p className="empty">当前时间范围还没有历史采样数据。</p>}</div>
    </section></div>}
    {timelineOpen && <div className="modal-backdrop" onClick={() => setTimelineOpen(false)}><section className="modal report-modal timeline-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">本地保存的最近事件</span><h3>🐾 全部巡逻记录</h3></div><button className="modal-close" onClick={() => setTimelineOpen(false)} aria-label="关闭巡逻记录">×</button></div>
      {status.timeline.length ? <VirtualList items={status.timeline} itemHeight={74} className="timeline timeline-full" keyFor={item => item.id} renderItem={item => <li className={`timeline-${item.kind}`}><time>{formatFullDateTime(item.occurredAt)}</time><div><b>{timelineKindLabel[item.kind] ?? "事件"}</b><span>{item.message}</span></div></li>} /> : <p className="empty">还没有巡逻记录。</p>}
    </section></div>}
    {appHistory && <div className="modal-backdrop" onClick={closeApplicationHistory}><section className="modal report-modal app-history-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">单个应用资源轨迹</span><h3>{appHistory.name} 历史曲线</h3></div><button className="modal-close" onClick={closeApplicationHistory} aria-label={appHistoryReturnApp == null ? "关闭应用历史" : "返回应用详情"} title={appHistoryReturnApp == null ? "关闭" : "返回应用详情"}>{appHistoryReturnApp == null ? "×" : "←"}</button></div>
      <div className="range-tabs metric-history-ranges">{[[10,"10 分钟"],[60,"1 小时"],[1440,"24 小时"],[10080,"7 天"]].map(([range,label]) => <button className={appHistoryRange === range ? "active" : ""} key={range} onClick={() => void showApplicationHistory(appHistory.name, Number(range))}>{label}</button>)}</div>
      <div className="report-scroll"><div className="app-history-charts">{([['cpuPercent','CPU'],['memoryBytes','内存'],['diskReadBps','磁盘读取'],['diskWriteBps','磁盘写入']] as const).map(([key,label]) => <ApplicationHistoryChart key={key} history={appHistory} metric={key} label={label} />)}</div>{!appHistory.points.length && <p className="empty">该应用在所选范围内还没有历史数据，保持运行一会儿后会自动积累。</p>}</div>
    </section></div>}
    {appDetails && appDetailsPresentation && <div className="modal-backdrop" onClick={() => setSelectedApp(null)}><section className="modal report-modal app-detail-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">应用实时详情 · {appDetails.name}</span><h3>{appDetailsPresentation.productName}</h3></div><button className="modal-close" onClick={() => setSelectedApp(null)} aria-label="关闭应用详情">×</button></div>
      <div className="report-scroll" ref={appDetailScrollRef} onScroll={event => { appDetailScrollTop.current = event.currentTarget.scrollTop; }}>
        <div className="app-detail-intro"><p>{appDetailsPresentation.description}</p><div><span>发布者 <b>{appDetails.publisher || "程序文件未提供"}</b></span><span>程序位置 <code title={appDetails.executablePath || ""}>{appDetails.executablePath || "当前权限无法读取"}</code></span></div></div>
        <div className="app-detail-summary">
          <article><span>CPU 总占用</span><b>{appDetails.cpuPercent.toFixed(1)}%</b></article>
          <article><span>内存总占用</span><b>{formatBytes(appDetails.memoryBytes)}</b></article>
          <article><span>磁盘读取</span><b>{formatRate(appDetails.diskReadBps ?? 0)}</b></article>
          <article><span>磁盘写入</span><b>{formatRate(appDetails.diskWriteBps ?? 0)}</b></article>
          <article><span>网络下载</span><b>{appDetails.networkReceiveBps == null ? "--" : formatRate(appDetails.networkReceiveBps)}</b></article>
          <article><span>网络上传</span><b>{appDetails.networkSendBps == null ? "--" : formatRate(appDetails.networkSendBps)}</b></article>
          <article><span>进程 / 线程</span><b>{appDetails.memberCount} / {appDetails.members.reduce((sum, process) => sum + (process.threadCount ?? 0), 0)}</b></article>
          <article><span>句柄总数</span><b>{appDetails.members.reduce((sum, process) => sum + (process.handleCount ?? 0), 0)}</b></article>
        </div>
        <div className="app-detail-identity"><span>根进程 PID <b>{appDetails.rootPid}</b></span><span>启动于 <b>{new Date(appDetails.rootProcess.startedAt * 1000).toLocaleString("zh-CN")}</b></span><span>{appDetails.rootProcess.isCritical ? "Windows 关键进程" : "普通应用进程"}</span><button onClick={() => { setAppHistoryReturnApp(appDetails.rootPid); void showApplicationHistory(appDetails.name); }}>查看历史曲线</button></div>
        <div className="app-process-detail-head"><h4>包含的进程</h4><span>点击进程可查看更多操作</span></div>
        <div className="app-process-detail-list">
          <div className="app-process-detail-row head"><b>进程</b><b>PID</b><b>启动时间</b><b>CPU</b><b>内存</b><b>磁盘读 / 写</b><b>线程 / 句柄</b></div>
          {appDetails.members.map(process => <button className="app-process-detail-row" key={`${process.pid}-${process.startedAt}`} onClick={() => { setSelectedApp(null); void inspect(process); }}>
            <span title={process.name}>{process.name}</span><span>{process.pid}</span><time title={new Date(process.startedAt * 1000).toLocaleString("zh-CN")}>{new Date(process.startedAt * 1000).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" })}</time><span>{process.cpuPercent.toFixed(1)}%</span><span>{formatBytes(process.memoryBytes)}</span><span>{formatRate(process.diskReadBps ?? 0)} / {formatRate(process.diskWriteBps ?? 0)}</span><span>{process.threadCount ?? 0} / {process.handleCount ?? "--"}</span>
          </button>)}
        </div>
      </div>
    </section></div>}
    {alerts && <div className="modal-backdrop" onClick={() => setAlerts(null)}><section className="modal report-modal alert-center" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">持久化处理记录</span><h3>🔔 告警中心</h3></div><button className="modal-close" onClick={() => setAlerts(null)} aria-label="关闭告警中心">×</button></div>
      <div className="report-filters">{[["","全部"],["unread","未读"],["acknowledged","已确认"],["ignored","已忽略"],["resolved","已恢复"]].map(([key,label]) => <button key={key} className={alertFilter === key ? "active" : ""} onClick={() => void showAlerts(key)}>{label}</button>)}</div>
      <div className="report-scroll alert-list">{alerts.map(alert => <article key={alert.id} className={`alert-row ${alert.severity}`}><div><span>{alert.status}</span><b>{alert.title}</b><p>{alert.message}</p><small>{new Date(alert.firstSeenAt).toLocaleString("zh-CN")}{alert.note && ` · 备注：${alert.note}`}</small></div><div><button onClick={() => void changeAlert(alert,"acknowledged")}>确认</button><button onClick={() => void changeAlert(alert,"ignored")}>忽略</button></div></article>)}{!alerts.length && <p className="empty">当前筛选条件下没有告警。</p>}</div>
    </section></div>}
    {patterns && <div className="modal-backdrop" onClick={() => setPatterns(null)}><section className="modal report-modal pattern-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">最近 30 天本地分析</span><h3>🕒 周期规律</h3></div><button className="modal-close" onClick={() => setPatterns(null)} aria-label="关闭周期规律">×</button></div>
      <div className="report-scroll pattern-list">{patterns.map(pattern => <article key={pattern.hour}><b>{String(pattern.hour).padStart(2,"0")}:00–{String((pattern.hour+1)%24).padStart(2,"0")}:00</b><span>{pattern.signal}</span><small>CPU {pattern.averageCpuPercent.toFixed(1)}% · 内存 {pattern.averageMemoryPercent.toFixed(1)}% · {pattern.sampleCount} 个样本</small></article>)}{!patterns.length && <p className="empty">暂未发现稳定的周期性高负载，需要积累更多跨天样本。</p>}</div>
    </section></div>}
    {cleanup && <div className="modal-backdrop" onClick={() => setCleanup(null)}><section className="modal report-modal cleanup-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">只读扫描 · 删除前再次确认</span><h3>🧹 磁盘清理助手</h3></div><button className="modal-close" onClick={() => setCleanup(null)} aria-label="关闭清理助手">×</button></div>
      <div className="cleanup-summary"><b>可安全选择 {formatBytes(cleanup.reclaimableBytes)}</b><span>选中的缓存将移入 Windows 回收站；系统缓存、回收站容量和重复文件只做分析。</span><button disabled={!cleanupSelection.size} onClick={() => void cleanSelected()}>清理已选 {cleanupSelection.size} 项</button></div>
      <div className="cleanup-filters" aria-label="清理分类"><button className={cleanupFilter === "all" ? "active" : ""} onClick={() => setCleanupFilter("all")}>全部 {cleanup.candidates.length}</button>{cleanupCategories.map(category => <button key={category} className={cleanupFilter === category ? "active" : ""} onClick={() => setCleanupFilter(category)}>{category}</button>)}</div>
      {visibleCleanup.length ? <VirtualList items={visibleCleanup} itemHeight={62} className="cleanup-list" keyFor={item => `${item.category}-${item.path}`} renderItem={item => <label><input type="checkbox" disabled={!item.cleanable} checked={item.cleanable && cleanupSelection.has(item.path)} onChange={event => setCleanupSelection(current => { const next = new Set(current); if (event.target.checked) next.add(item.path); else next.delete(item.path); return next; })}/><div><b>{item.category}</b><code title={item.path}>{item.path}</code><small>{formatBytes(item.sizeBytes)} · {new Date(item.modifiedAt).toLocaleString("zh-CN")}</small></div><span>{item.cleanable ? "可回收" : "只读分析"}</span></label>} /> : <p className="empty">当前分类没有发现可展示的项目。</p>}
    </section></div>}
    {storageOpen && snap && <div className="modal-backdrop" onClick={() => setStorageOpen(false)}><section className="modal report-modal storage-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">实时容量分析</span><h3>💾 存储分析</h3></div><button className="modal-close" onClick={() => setStorageOpen(false)} aria-label="关闭存储分析">×</button></div>
      <p className="security-note">选择磁盘后会读取文件系统元数据，统计所有可访问文件与文件夹的大小；不会打开或读取文件内容。</p>
      <div className="report-scroll"><Suspense fallback={<p className="empty">正在整理磁盘数据…</p>}><StorageAnalysis disks={snap.hardware.disks} onActivity={setDogActivity} /></Suspense></div>
    </section></div>}
    {hardwareOpen && snap && <div className="modal-backdrop" onClick={() => setHardwareOpen(false)}><section className="modal report-modal hardware-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">实时设备指标</span><h3>🖥️ 硬件监控</h3></div><button className="modal-close" onClick={() => setHardwareOpen(false)} aria-label="关闭硬件监控">×</button></div>
      <div className="report-scroll"><div className="report-tabs hardware-tabs" role="tablist">{([['cpu','CPU 核心'],['gpu','GPU'],['power','电池与传感器'],['disks','磁盘分区'],['network','网络适配器'],['apps','应用资源']] as const).map(([key,label]) => <button key={key} className={hardwareTab === key ? "active" : ""} onClick={() => setHardwareTab(key)}>{label}</button>)}</div>
      {hardwareTab === "cpu" && <div className="hardware-grid">{snap.hardware.cpuCores.map(core => <article key={core.name}><span>{core.name}</span><b>{core.usagePercent.toFixed(1)}%</b><i><em style={{width: `${core.usagePercent}%`}} /></i><small>{core.frequencyMhz} MHz</small></article>)}</div>}
      {hardwareTab === "gpu" && <div>{snap.hardware.gpus.map(gpu => <article className="hardware-row" key={gpu.name}><div><b>{gpu.name}</b><small>驱动实时指标</small></div><span>使用率 {gpu.usagePercent.toFixed(1)}%</span><span>显存 {formatBytes(gpu.memoryUsedBytes)} / {formatBytes(gpu.memoryTotalBytes)}</span></article>)}{!snap.hardware.gpus.length && <p className="availability">{snap.hardware.gpuStatus}</p>}</div>}
      {hardwareTab === "power" && <div className="sensor-section">{snap.hardware.battery ? <article className="hardware-row"><div><b>电池 {snap.hardware.battery.chargePercent}%</b><small>{snap.hardware.battery.charging ? "正在充电" : snap.hardware.battery.acConnected ? "已连接电源" : "正在放电"}</small></div><span>{snap.hardware.battery.lifeSeconds ? `预计 ${formatDuration(snap.hardware.battery.lifeSeconds)}` : "剩余时间未知"}</span></article> : <p className="availability">未检测到电池，台式机通常没有此项。</p>}<h4>温度</h4>{snap.hardware.temperatures.map(sensor => <article className="hardware-row" key={sensor.label}><b>{sensor.label}</b><span>{sensor.celsius.toFixed(1)}°C{sensor.maxCelsius ? ` · 峰值 ${sensor.maxCelsius.toFixed(1)}°C` : ""}</span></article>)}{!snap.hardware.temperatures.length && <p className="availability">硬件或驱动未向 Windows 提供温度传感器。</p>}<h4>风扇</h4>{snap.hardware.fans.map(fan => <article className="hardware-row" key={fan.label}><b>{fan.label}</b><span>{fan.rpm} RPM</span></article>)}{!snap.hardware.fans.length && <p className="availability">{snap.hardware.fanStatus}</p>}</div>}
      {hardwareTab === "disks" && <div>{snap.hardware.disks.map(disk => <article className="hardware-row" key={`${disk.name}-${disk.mountPoint}`}><div><b>{disk.name || "本地磁盘"} · {disk.mountPoint}</b><small>可用 {formatBytes(disk.availableBytes)} / {formatBytes(disk.totalBytes)}</small></div><span>读 {formatRate(disk.readBps)}</span><span>写 {formatRate(disk.writeBps)}</span></article>)}</div>}
      {hardwareTab === "network" && <div>{snap.hardware.networks.map(adapter => <article className="hardware-row" key={adapter.name}><b>{adapter.name}</b><span>下载 {formatRate(adapter.receivedBps)}</span><span>上传 {formatRate(adapter.transmittedBps)}</span></article>)}</div>}
      {hardwareTab === "apps" && <div><p className="availability">{snap.hardware.appNetworkStatus}</p>{snap.applications.map(app => <article className="hardware-row" key={`${app.rootPid}-${app.name}`}><div><b>{app.name}</b><small>{app.memberCount} 进程 · {app.members.reduce((sum,item) => sum + (item.handleCount ?? 0), 0)} 句柄</small></div><span>磁盘读 {formatRate(app.diskReadBps ?? 0)}</span><span>磁盘写 {formatRate(app.diskWriteBps ?? 0)}</span><span>{app.networkBps == null ? "网络 --" : `网络下 ${formatRate(app.networkReceiveBps ?? 0)} / 上 ${formatRate(app.networkSendBps ?? 0)}`}</span></article>)}</div>}
      </div>
    </section></div>}
    {security && <div className="modal-backdrop" onClick={() => setSecurity(null)}><section className="modal report-modal security-report" onClick={e => e.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">只读安全扫描</span><h3>🛡️ 看门报告</h3></div><button className="modal-close" onClick={() => setSecurity(null)} aria-label="关闭看门报告">×</button></div>
      <div className="report-scroll">
        <div className="report-tabs" role="tablist" aria-label="安全报告分类">
          {([['overview','概览'],['programs','运行程序'],['startup','启动项'],['network','网络连接'],['tasks','计划任务'],['services','Windows 服务']] as const).map(([key, label]) => <button key={key} role="tab" aria-selected={securityTab === key} className={securityTab === key ? "active" : ""} onClick={() => setSecurityTab(key)}>{label}</button>)}
        </div>
        {securityTab === "overview" && <div className="security-tab-panel"><div className="security-scoreboard">
          <div className={`security-score ${security.securityScore < 70 ? "attention" : ""}`}><strong>{security.securityScore}</strong><span>安全分</span></div>
          <div><b>{security.mediumRiskCount}</b><span>需要确认</span></div>
          <div><b>{security.lowRiskCount}</b><span>低风险信号</span></div>
          <div><b>{security.signedPrograms}/{security.scannedPrograms}</b><span>签名有效</span></div>
        </div>
        <div className="security-inventory"><span><b>{security.networkConnections.length}</b> TCP 连接</span><span><b>{security.scheduledTasks.length}</b> 计划任务</span><span><b>{security.windowsServices.length}</b> Windows 服务</span></div>
        <div className="security-summary"><b>{security.summary}</b><span>扫描于 {new Date(security.scannedAt).toLocaleString("zh-CN")} · {security.startupEntries.length} 个启动项</span></div>
        <p className="security-note">未验证不等于恶意程序。大黄狗只展示客观信号，请结合来源和用途判断。</p>
        </div>}
        {securityTab === "programs" && <div className="security-tab-panel">
        <div className="report-filters"><button className={securityFilter === "all" ? "active" : ""} onClick={() => setSecurityFilter("all")}>全部</button><button className={securityFilter === "medium" ? "active" : ""} onClick={() => setSecurityFilter("medium")}>需要确认</button><button className={securityFilter === "low" ? "active" : ""} onClick={() => setSecurityFilter("low")}>低风险</button></div>
        {visiblePrograms.length > 0 && <div className="security-group"><h4>运行中的程序</h4>{visiblePrograms.map(program => <article className="security-item" key={`${program.pid}-${program.path}`}>
          <span className={`risk-pill ${program.riskLevel}`}>{program.riskLevel === "medium" ? "需确认" : "低风险"}</span><div><b>{program.name}</b><code title={program.path}>{program.path}</code><small>{program.reasons.length ? program.reasons.join(" · ") : "未发现额外风险信号"}</small></div><span>{program.signatureStatus === "valid" ? "✓ 签名有效" : "? 签名未验证"}</span><button className="reveal-file" onClick={() => revealSecurityFile(program.path)} title="在资源管理器中定位文件">打开位置</button>
        </article>)}</div>}
        {!visiblePrograms.length && <p className="empty">当前筛选条件下没有程序。</p>}</div>}
        {securityTab === "startup" && <div className="security-tab-panel security-group"><h4>开机启动项</h4>{security.startupEntries.length ? security.startupEntries.map(entry => <article className="security-item startup" key={`${entry.source}-${entry.name}`}>
          <span className={`risk-pill ${entry.riskLevel}`}>{entry.source === "大黄狗\\已禁用" ? "已禁用" : entry.riskLevel === "medium" ? "需确认" : "正常"}</span><div><b>{entry.name}</b><code>{entry.command}</code><small>{entry.source}{entry.reasons.length ? ` · ${entry.reasons.join(" · ")}` : ""}</small></div>{(entry.source.startsWith("HKEY_CURRENT_USER") || entry.source === "大黄狗\\已禁用") && <button className="reveal-file" onClick={() => void changeStartup(entry)}>{entry.source === "大黄狗\\已禁用" ? "启用" : "禁用"}</button>}
        </article>) : <p className="empty">没有读取到常见启动项。</p>}</div>}
        {securityTab === "network" && <div className="security-tab-panel security-group"><h4>监听端口与网络连接</h4>{security.networkConnections.map(connection => <article className="inventory-row" key={`${connection.protocol}-${connection.localAddress}-${connection.remoteAddress}-${connection.pid}`}><b>{connection.processName}</b><code>{connection.localAddress} → {connection.remoteAddress}</code><span>PID {connection.pid} · {connection.state}</span></article>)}{!security.networkConnections.length && <p className="empty">没有读取到 TCP 连接。</p>}</div>}
        {securityTab === "tasks" && <div className="security-tab-panel security-group"><h4>计划任务</h4>{security.scheduledTasks.map(task => <article className="inventory-row" key={task.path}><b>{task.name}</b><code>{task.path}</code></article>)}{!security.scheduledTasks.length && <p className="empty">没有读取到计划任务，部分目录可能需要管理员权限。</p>}</div>}
        {securityTab === "services" && <div className="security-tab-panel security-group"><h4>Windows 服务</h4>{security.windowsServices.map(service => <article className="inventory-row" key={service.name}><b>{service.name}</b><code>{service.imagePath || "系统驱动或未设置路径"}</code><span>{service.startMode}</span></article>)}</div>}
      </div>
    </section></div>}
    {usage && <div className="modal-backdrop" onClick={() => setUsage(null)}><section className="modal report-modal usage-report" onClick={e => e.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">应用生命周期</span><h3>⏱ 使用记录</h3></div><button className="modal-close" onClick={() => setUsage(null)} aria-label="关闭使用记录">×</button></div>
      <div className="usage-content report-scroll">
      <p className="security-note">启动与运行时间来自进程生命周期；前台使用时间从大黄狗首次观察后累计。</p>
      <div className="usage-period" aria-label="统计时间范围">{[[7,"近 7 天"],[30,"近 30 天"],[90,"近 90 天"]].map(([period,label]) => <button key={period} className={usagePeriod === period ? "active" : ""} disabled={busy} onClick={() => changeUsagePeriod(Number(period))}>{label}</button>)}</div>
      <div className="usage-tabs" role="tablist" aria-label="使用记录视图">
        <button role="tab" aria-selected={usageTab === "charts"} className={usageTab === "charts" ? "active" : ""} onClick={() => setUsageTab("charts")}>图形统计</button>
        <button role="tab" aria-selected={usageTab === "list"} className={usageTab === "list" ? "active" : ""} onClick={() => setUsageTab("list")}>明细列表</button>
      </div>
      {usageTab === "charts" && usageSummary && <div className="usage-tab-panel" role="tabpanel">
        <div className="usage-summary">
          <div><b>{formatDuration(usageSummary.totalForegroundSeconds)}</b><span>近 {usageSummary.periodDays} 天前台使用</span></div>
          <div><b>{formatDuration(usageSummary.totalBackgroundSeconds)}</b><span>后台运行</span></div>
          <div><b>{usageSummary.applicationCount}</b><span>使用过的应用</span></div>
          <div><b>{usageSummary.longestUsedApp ?? "暂无"}</b><span>最常使用</span></div>
          <div><b>{usageSummary.sessionCount}</b><span>应用启动次数</span></div>
          <div><b>{formatDuration(Math.round(usageSummary.totalForegroundSeconds / Math.max(1, usageSummary.sessionCount)))}</b><span>平均单次前台使用</span></div>
        </div>
        {usageSummary.topApps.length > 0 && <div className="usage-ranking"><h4>前台使用排行</h4>{usageSummary.topApps.slice(0, 5).map((app, index) => <div key={app.name}><span>{index + 1}. {app.name}</span><i><em style={{width: `${Math.max(4, app.foregroundSeconds / Math.max(1, usageSummary.topApps[0].foregroundSeconds) * 100)}%`}} /></i><b>{formatDuration(app.foregroundSeconds)}</b></div>)}<div className="usage-ranking-axis"><span>0</span><span>{formatDuration(Math.round(usageSummary.topApps[0].foregroundSeconds / 2))}</span><span>{formatDuration(usageSummary.topApps[0].foregroundSeconds)}</span></div><small className="usage-axis-title">X：前台使用时长 · Y：应用</small></div>}
        {usageSummary.dailyUsage.length > 0 && <div className="daily-usage"><h4>每日使用与启动次数</h4><DailyUsageChart days={usageSummary.dailyUsage} /></div>}
      </div>}
      {usageTab === "list" && <div className="usage-tab-panel usage-list-panel" role="tabpanel">
      <div className="usage-list-tools"><input className="usage-search" value={usageQuery} onChange={event => setUsageQuery(event.target.value)} placeholder="搜索应用名称" /><button onClick={exportUsage}>导出 CSV</button></div>
      <div className="usage-head"><span>应用</span><span>启动 / 关闭</span><span>运行时间</span><span>前台使用</span></div>
      {visibleUsage.length ? <VirtualList items={visibleUsage} itemHeight={72} className="usage-list" keyFor={record => record.sessionId} renderItem={record => <article className="usage-row">
        <div><b>{record.name}</b><small>PID {record.rootPid} · 峰值 {record.memberPeak} 个进程</small></div>
        <div><span>{new Date(record.startedAt).toLocaleString("zh-CN")}</span><small>{record.isRunning ? "仍在运行" : record.closedAt ? `关闭于 ${new Date(record.closedAt).toLocaleString("zh-CN")}` : "关闭时间未知"}</small></div>
        <div><b>{formatDuration(record.runtimeSeconds)}</b><small>后台 {formatDuration(record.backgroundSeconds)}</small></div>
        <div><b>{formatDuration(record.foregroundSeconds)}</b><small>{record.isRunning ? "● 活跃会话" : "已结束"}</small></div>
      </article>} /> : <p className="empty">没有匹配的应用使用记录。</p>}
      </div>}
      </div>
    </section></div>}
    {selected && <div className="modal-backdrop" onClick={() => setSelected(null)}><section className="modal process-actions-modal" onClick={e => e.stopPropagation()}>
      <span className="risk">进程操作</span><h3>{selected.name}</h3><p>PID {selected.pid} · CPU {selected.cpuPercent.toFixed(1)}% · {formatBytes(selected.memoryBytes)}</p>
      <div className="process-action-list"><button onClick={() => { setAppHistoryReturnApp(null); void showApplicationHistory(selected.name); }}>📈 查看历史曲线 <small>只读操作</small></button><button onClick={() => reveal(selected)}>📂 打开文件位置 <small>只读操作</small></button><button onClick={() => requestPriority(selected, "belowNormal")}>⬇ 调低优先级 <small>需要确认</small></button><button onClick={() => requestPriority(selected, "normal")}>↔ 恢复正常优先级 <small>需要确认</small></button><button className="danger-row" onClick={() => requestTerminate(selected)}>结束进程 <small>可能丢失未保存内容</small></button></div>
      <div className="actions"><button className="secondary" onClick={() => setSelected(null)}>取消</button></div>
    </section></div>}
    <nav className="bottom-menu" aria-label="底部菜单">
      <div className="bottom-menu-inner">
        <div className={`live live-${status.dogState}`} role="status" aria-live="polite"><i /><span>{stateLabel[status.dogState]}</span></div>
        <button className="bottom-settings" onClick={() => setSettingsOpen(true)} aria-label="打开设置"><span>⚙️</span><b>设置</b></button>
      </div>
    </nav>

    {diagnosisOpen && <div className="modal-backdrop" onPointerDown={closeDiagnosis}><section className="modal report-modal diagnosis-modal" onPointerDown={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">资源状态智能分析</span><h3>🐕 {diagnosis?.source === "minimax" ? "MiniMax AI 诊断" : diagnosis?.source === "local-fallback" ? "本地诊断（AI 已降级）" : "电脑卡顿诊断"}</h3></div><button type="button" className="modal-close diagnosis-close" onPointerDown={event => { event.stopPropagation(); closeDiagnosis(); }} onClick={closeDiagnosis} aria-label={diagnosisLoading ? "关闭诊断，分析将在后台继续" : "关闭诊断"}>×</button></div>
      <div className="report-scroll">{diagnosisLoading ? <div className="diagnosis-loading"><span className="storage-spinner"/><b>大黄正在分析当前资源状态…</b><p>MiniMax 分析通常需要几秒钟。</p></div> : diagnosis && <div className="diagnosis diagnosis-dialog-content">
        <h2>{diagnosis.summary}</h2><div className="diagnosis-grid"><div><b>我看到的</b><ul>{diagnosis.details.map(item => <li key={item}>{item}</li>)}</ul></div><div><b>我的建议</b><ul>{diagnosis.suggestions.map(item => <li key={item}>{item}</li>)}</ul></div></div>
        <small>置信度：{diagnosis.confidence === "high" ? "高" : diagnosis.confidence === "medium" ? "中" : "低"} · {diagnosis.source === "minimax" ? `由 ${diagnosis.model} 分析，仅发送资源摘要` : "使用本机规则分析"}</small>
      </div>}</div>
    </section></div>}
    {settingsOpen && settings && <div className="modal-backdrop" onClick={closeSettings}><section className="modal settings-modal" onClick={e => e.stopPropagation()}>
      <header className="settings-header">
        <div><span className="risk">设置中心</span><h3>大黄狗设置</h3><p>调整巡逻频率、后台能力和 AI 诊断。</p></div>
        <button className="modal-close" onClick={closeSettings} aria-label="关闭设置">×</button>
      </header>
      <nav className="settings-tabs" role="tablist" aria-label="设置分类">
        {([['patrol','巡逻与记录'],['companion','陪伴体验'],['system','后台能力'],['ai','MiniMax AI'],['data','数据管理']] as const).map(([key,label]) => <button key={key} role="tab" aria-selected={settingsTab === key} className={settingsTab === key ? "active" : ""} onClick={() => setSettingsTab(key)}>{label}</button>)}
      </nav>
      <div className="settings-content">
        {settingsTab === "patrol" && <section className="settings-section settings-tab-panel">
          <div className="settings-section-title"><div><h4>巡逻与记录</h4><p>控制异常阈值、采样速度和历史数据保留周期。</p></div><span>基础</span></div>
          <div className="settings-grid">
            <label className="setting-field setting-range">CPU 告警阈值 <output>{settings.cpuThreshold}%</output><input type="range" min="70" max="99" value={settings.cpuThreshold} onChange={e => setSettings({...settings, cpuThreshold: Number(e.target.value)})} /></label>
            <label className="setting-field setting-range">内存告警阈值 <output>{settings.memoryThreshold}%</output><input type="range" min="70" max="99" value={settings.memoryThreshold} onChange={e => setSettings({...settings, memoryThreshold: Number(e.target.value)})} /></label>
            <label className="setting-field">普通采样间隔 <select value={settings.samplingSeconds} onChange={e => setSettings({...settings, samplingSeconds: Number(e.target.value)})}><option value="2">2 秒</option><option value="5">5 秒</option><option value="10">10 秒</option><option value="30">30 秒</option></select></label>
            <label className="setting-field">历史保留天数 <select value={settings.retentionDays} onChange={e => setSettings({...settings, retentionDays: Number(e.target.value)})}><option value="1">1 天</option><option value="7">7 天</option><option value="30">30 天</option><option value="90">90 天</option></select></label>
          </div>
        </section>}
        {settingsTab === "companion" && <section className="settings-section settings-tab-panel">
          <div className="settings-section-title"><div><h4>陪伴体验</h4><p>调整大黄的回应方式；不会影响巡逻和安全功能。</p></div><span>舒心</span></div>
          <div className="settings-grid">
            <label className="setting-field">小狗性格 <select value={settings.companionPersonality} onChange={e => setSettings({...settings, companionPersonality: e.target.value as UserSettings["companionPersonality"]})}><option value="quiet">安静</option><option value="warm">温暖</option><option value="playful">活泼</option></select></label>
          </div>
          <div className="settings-toggles companion-settings">
            <label className="check"><input type="checkbox" checked={settings.companionQuietMode} onChange={e => setSettings({...settings, companionQuietMode: e.target.checked})}/><span><b>安静模式</b><small>不主动显示气泡；点击小狗时仍会回应。</small></span></label>
            <label className="check"><input type="checkbox" checked={settings.reduceCompanionMotion} onChange={e => setSettings({...settings, reduceCompanionMotion: e.target.checked})}/><span><b>减少动画</b><small>保留状态反馈，停止摇尾巴、呼吸和庆祝动作。</small></span></label>
          </div>
        </section>}
        {settingsTab === "system" && <section className="settings-section settings-tab-panel">
          <div className="settings-section-title"><div><h4>后台能力</h4><p>选择大黄狗在 Windows 中可以执行的巡逻任务。</p></div><span>系统</span></div>
          <div className="settings-toggles">
            <label className="check"><input type="checkbox" checked={settings.lowPowerMode} onChange={e => setSettings({...settings, lowPowerMode: e.target.checked})} /><span><b>低功耗模式</b><small>固定每 15 秒巡逻，降低后台资源占用。</small></span></label>
            <label className="check"><input type="checkbox" checked={settings.notificationsEnabled} onChange={e => setSettings({...settings, notificationsEnabled: e.target.checked})} /><span><b>Windows 异常通知</b><small>发现需要关注的问题时发送系统通知。</small></span></label>
            <label className="check"><input type="checkbox" checked={settings.autoStart} onChange={e => setSettings({...settings, autoStart: e.target.checked})} /><span><b>登录后自动巡逻</b><small>登录 Windows 后自动启动并驻留托盘。</small></span></label>
            <label className="check"><input type="checkbox" checked={settings.applicationNetworkMonitoring} onChange={e => setSettings({...settings, applicationNetworkMonitoring: e.target.checked})} /><span><b>应用级网络流量</b><small>需要管理员权限启动 ETW 采集会话。</small></span></label>
          </div>
          <p className="availability">网络监控只记录每个进程的收发字节数，不采集网络内容；权限不足时数据会保持为空。</p>
        </section>}
        {settingsTab === "ai" && <section className="settings-section ai-settings settings-tab-panel">
          <div className="settings-section-title"><div><h4>MiniMax AI 诊断</h4><p>用当前资源指标和异常摘要分析电脑卡顿原因。</p></div><span className={aiConfigured ? "configured" : ""}>{aiConfigured ? "已配置" : "未配置"}</span></div>
          <label className="check ai-toggle"><input type="checkbox" checked={settings.minimaxEnabled} onChange={e => setSettings({...settings, minimaxEnabled: e.target.checked})} /><span><b>启用 MiniMax 分析</b><small>用于“大黄，电脑为什么卡？”诊断。</small></span></label>
          <div className="settings-grid ai-fields">
            <label className="setting-field">模型 <select value={settings.minimaxModel} onChange={e => setSettings({...settings, minimaxModel: e.target.value})}><option value="MiniMax-M2.7">MiniMax-M2.7</option><option value="MiniMax-M2.7-highspeed">MiniMax-M2.7 高速</option><option value="MiniMax-M2.5">MiniMax-M2.5</option><option value="MiniMax-M2.5-highspeed">MiniMax-M2.5 高速</option></select></label>
            <label className="setting-field">API Key <input type="password" value={minimaxKey} onChange={e => { setMinimaxKey(e.target.value); setAiTestResult(null); }} autoComplete="off" placeholder={aiConfigured ? "••••••••••••••••" : "粘贴 MiniMax API Key"} aria-label={aiConfigured ? "API Key 已保存，输入新 Key 可替换" : "输入 MiniMax API Key"} title={aiConfigured ? "已安全保存；输入新 Key 可替换现有凭据" : undefined} /></label>
          </div>
          <div className="ai-test-row"><button disabled={aiTesting} onClick={() => void testMinimax()}>{aiTesting ? "正在测试…" : "测试连接"}</button>{aiTestResult && <span className={aiTestResult.ok ? "success" : "failure"}>{aiTestResult.ok ? "✓" : "×"} {aiTestResult.message}</span>}</div>
          <p className="ai-privacy">🔒 密钥保存在 Windows 凭据管理器。AI 不会接收文件路径、PID 或文件内容。</p>
          {aiConfigured && <button className="remove-ai-key" onClick={() => void removeMinimaxKey()}>删除已保存的 API Key</button>}
        </section>}
        {settingsTab === "data" && <div className="settings-tab-panel"><section className="settings-section data-settings-intro"><div className="settings-section-title"><div><h4>本地数据管理</h4><p>管理大黄狗保存在此电脑上的历史数据。</p></div><span>本机</span></div><p className="availability">历史快照、巡逻记录和动作审计仅保存在本机。清除后无法恢复。</p></section><section className="settings-danger"><div><b>清除本地记忆</b><span>删除历史快照、巡逻记录和动作审计，且无法撤销。</span></div><button onClick={clearMemory}>清除数据</button></section></div>}
      </div>
      <footer className="settings-footer"><span>更改只会在点击保存后生效</span><div className="actions"><button className="secondary" onClick={closeSettings}>取消</button><button className="primary" disabled={busy} onClick={persistSettings}>{busy ? "正在保存…" : "保存设置"}</button></div></footer>
    </section></div>}
    {preview && <div className="modal-backdrop" onClick={() => setPreview(null)}><section className="modal" onClick={e => e.stopPropagation()}>
      <span className="risk">{preview.riskLevel} · 需要确认</span><h3>{preview.title}</h3><p>{preview.warning}</p>
      <div className="target"><b>{preview.target.name}</b><span>PID {preview.target.pid} · CPU {preview.target.cpuPercent.toFixed(1)}% · {formatBytes(preview.target.memoryBytes)}</span></div>
      {!preview.allowed && <p className="blocked">为了系统安全，大黄狗拒绝执行这个操作。</p>}
      <div className="actions"><button className="secondary" onClick={() => setPreview(null)}>先不处理</button>{preview.allowed && <button className={preview.action === "terminateProcess" ? "danger" : "primary"} disabled={busy} onClick={execute}>{busy ? "正在处理…" : preview.action === "terminateProcess" ? "确认结束进程" : "确认调整优先级"}</button>}</div>
    </section></div>}
  </main>;
}
