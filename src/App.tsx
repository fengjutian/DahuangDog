import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { clearLocalMemory, confirmAction, diagnosePerformance, exportUsageCsv, getAppUsageHistory, getAppUsageSummary, getCurrentStatus, getHistoryRange, getSecurityReport, getSettings, openFileLocation, openProcessLocation, preparePriority, prepareTerminate, saveSettings } from "./api";
import type { ActionPreview, AppUsageRecord, AppUsageSummary, CurrentStatus, HistorySummary, LocalDiagnosis, MetricPoint, ProcessSample, SecurityReport, UserSettings } from "./types";

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

type HistoryMetricKey = "cpuPercent" | "memoryPercent" | "diskBps" | "networkBps";

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

function Metric({ label, value, tone, onClick }: { label: string; value: string; tone?: "warn"; onClick?: () => void }) {
  if (onClick) return <button className={`metric metric-clickable ${tone ?? ""}`} onClick={onClick} aria-label={`查看${label}历史明细`}><span>{label}</span><strong>{value}</strong><small>查看历史</small></button>;
  return <div className={`metric ${tone ?? ""}`}><span>{label}</span><strong>{value}</strong></div>;
}

function linePath(points: MetricPoint[], key: "cpuPercent" | "memoryPercent" | "diskBps" | "networkBps"): string {
  if (points.length < 2) return "";
  return points.map((point, index) => {
    const x = index / (points.length - 1) * 100;
    const max = key === "cpuPercent" || key === "memoryPercent" ? 100 : Math.max(...points.map(item => item[key]), 1);
    const y = 100 - Math.max(0, Math.min(100, point[key] / max * 100));
    return `${index ? "L" : "M"}${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
}

function TrendChart({ history, range, onRange }: { history: HistorySummary; range: number; onRange: (range: number) => void }) {
  return <section className="card trend-card">
    <div className="section-title"><h3>资源趋势</h3><div className="range-tabs">{[[10,"10 分钟"],[60,"1 小时"],[1440,"24 小时"],[10080,"7 天"]].map(([value,label]) => <button key={value} className={range === value ? "active" : ""} onClick={() => onRange(Number(value))}>{label}</button>)}</div></div>
    <div className="trend-grid">{([['cpuPercent','CPU','cpu-line'],['memoryPercent','内存','memory-line'],['diskBps','磁盘吞吐','disk-line'],['networkBps','网络吞吐','network-line']] as const).map(([key,label,style]) => <div className="mini-trend" key={key}><b>{label}</b><svg viewBox="0 0 100 100" preserveAspectRatio="none"><path className="grid-line" d="M0,25 H100 M0,50 H100 M0,75 H100"/><path className={style} d={linePath(history.points,key)}/></svg></div>)}</div>
    <div className="monitor-insights">
      <div><span>CPU 峰值</span><b>{history.peakCpuPercent.toFixed(0)}%</b></div>
      <div><span>内存峰值</span><b>{history.peakMemoryPercent.toFixed(0)}%</b></div>
      <div><span>平均磁盘</span><b>{formatRate(history.averageDiskBps)}</b></div>
      <div><span>平均网络</span><b>{formatRate(history.averageNetworkBps)}</b></div>
    </div>
  </section>;
}

export default function App() {
  const [status, setStatus] = useState<CurrentStatus | null>(null);
  const [preview, setPreview] = useState<ActionPreview | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<HistorySummary | null>(null);
  const [historyError, setHistoryError] = useState("");
  const [historyRange, setHistoryRange] = useState(10);
  const [diagnosis, setDiagnosis] = useState<LocalDiagnosis | null>(null);
  const [security, setSecurity] = useState<SecurityReport | null>(null);
  const [selected, setSelected] = useState<ProcessSample | null>(null);
  const [settings, setSettings] = useState<UserSettings | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [expandedApp, setExpandedApp] = useState<number | null>(null);
  const [usage, setUsage] = useState<AppUsageRecord[] | null>(null);
  const [usageSummary, setUsageSummary] = useState<AppUsageSummary | null>(null);
  const [usageQuery, setUsageQuery] = useState("");
  const [usageTab, setUsageTab] = useState<"charts" | "list">("charts");
  const [usagePeriod, setUsagePeriod] = useState(7);
  const [securityFilter, setSecurityFilter] = useState<"all" | "medium" | "low">("all");
  const [securityTab, setSecurityTab] = useState<"overview" | "programs" | "startup" | "network" | "tasks" | "services">("overview");
  const [hardwareOpen, setHardwareOpen] = useState(false);
  const [hardwareTab, setHardwareTab] = useState<"cpu" | "gpu" | "power" | "disks" | "network" | "apps">("cpu");
  const [processQuery, setProcessQuery] = useState("");
  const [timelineOpen, setTimelineOpen] = useState(false);
  const [selectedHistoryMetric, setSelectedHistoryMetric] = useState<HistoryMetricKey | null>(null);

  const refresh = useCallback(async () => {
    try { setStatus(await getCurrentStatus()); }
    catch (error) { setMessage(`暂时没听见系统消息：${String(error)}`); }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(refresh, 3000);
    let unlisten: undefined | (() => void);
    if ("__TAURI_INTERNALS__" in window) {
      void listen<CurrentStatus>("status://updated", event => setStatus(event.payload)).then(fn => { unlisten = fn; });
    }
    return () => { window.clearInterval(timer); unlisten?.(); };
  }, [refresh]);

  useEffect(() => {
    const load = () => void getHistoryRange(historyRange).then(result => { setHistory(result); setHistoryError(""); }).catch(error => setHistoryError(String(error)));
    load();
    const timer = window.setInterval(load, 10_000);
    return () => window.clearInterval(timer);
  }, [historyRange]);

  useEffect(() => { void getSettings().then(setSettings).catch(() => undefined); }, []);

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
    setBusy(true);
    try { setDiagnosis(await diagnosePerformance()); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
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
    try { setSettings(await saveSettings(settings)); setMessage("巡逻设置已经保存。" ); setSettingsOpen(false); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
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

  if (!status) return <main className="loading">🐕 大黄狗正在醒来……</main>;
  const snap = status.snapshot;
  const usageSince = Date.now() - usagePeriod * 24 * 60 * 60 * 1000;
  const visibleUsage = usage?.filter(record => record.lastSeenAt >= usageSince && record.name.toLowerCase().includes(usageQuery.trim().toLowerCase())) ?? [];
  const visiblePrograms = security?.programs.filter(program => securityFilter === "all" || program.riskLevel === securityFilter) ?? [];
  const normalizedProcessQuery = processQuery.trim().toLowerCase();
  const visibleApplications = snap?.applications.filter(application => {
    if (!normalizedProcessQuery) return true;
    return application.name.toLowerCase().includes(normalizedProcessQuery)
      || String(application.rootPid).includes(normalizedProcessQuery)
      || application.members.some(process => process.name.toLowerCase().includes(normalizedProcessQuery)
        || String(process.pid).includes(normalizedProcessQuery));
  }).slice(0, 8) ?? [];

  return <main className="shell">
    <header><div className="brand"><span className="dog">🐕</span><div><h1>大黄狗</h1><p>住在 Windows 里的 AI 看门狗</p></div></div><div className="header-actions"><button onClick={() => setHardwareOpen(true)}>🖥️ 硬件</button><button onClick={showUsage} disabled={busy}>⏱ 使用记录</button><button onClick={() => setSettingsOpen(true)}>⚙️ 设置</button><button onClick={scanSecurity} disabled={busy}>🛡️ 看门报告</button><span className="live"><i />{stateLabel[status.dogState]}</span></div></header>

    <section className="hero">
      <div className="avatar" aria-hidden="true">🐕</div>
      <div><span className="eyebrow">今天的巡逻报告</span><h2>{status.summary}</h2><p>健康度 <b>{status.healthScore}</b> / 100</p><button className="diagnose-button" onClick={diagnose} disabled={busy}>{busy ? "正在检查…" : "大黄，电脑为什么卡？"}</button></div>
    </section>

    <section className="metrics">
      <Metric label="CPU" value={snap ? `${snap.cpuPercent.toFixed(0)}%` : "--"} tone={snap && snap.cpuPercent >= 90 ? "warn" : undefined} onClick={() => setSelectedHistoryMetric("cpuPercent")} />
      <Metric label="内存" value={snap ? `${snap.memoryPercent.toFixed(0)}%` : "--"} tone={snap && snap.memoryPercent >= 90 ? "warn" : undefined} onClick={() => setSelectedHistoryMetric("memoryPercent")} />
      <Metric label="已用内存" value={snap ? formatBytes(snap.usedMemoryBytes) : "--"} />
      <Metric label="磁盘读 / 写" value={snap ? `${formatRate(snap.diskReadBps)} / ${formatRate(snap.diskWriteBps)}` : "--"} onClick={() => setSelectedHistoryMetric("diskBps")} />
      <Metric label="网络下 / 上" value={snap ? `${formatRate(snap.networkReceiveBps)} / ${formatRate(snap.networkSendBps)}` : "--"} onClick={() => setSelectedHistoryMetric("networkBps")} />
      <Metric label="发现" value={`${status.findings.length} 个`} tone={status.findings.length ? "warn" : undefined} />
      <Metric label="磁盘空间" value={snap ? `${formatBytes(snap.diskAvailableBytes)} 可用` : "--"} tone={snap && snap.diskTotalBytes > 0 && snap.diskAvailableBytes / snap.diskTotalBytes < .1 ? "warn" : undefined} />
      <Metric label="系统运行" value={snap ? formatDuration(snap.uptimeSeconds) : "--"} />
      <Metric label="进程 / 线程" value={snap ? `${snap.processes.length}+ / ${snap.processes.reduce((sum, process) => sum + (process.threadCount ?? 0), 0)}+` : "--"} />
    </section>

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

    {diagnosis && <section className="diagnosis card">
      <div className="section-title"><h3>🐕 本地诊断</h3><button onClick={() => setDiagnosis(null)}>收起</button></div>
      <h2>{diagnosis.summary}</h2>
      <div className="diagnosis-grid"><div><b>我看到的</b><ul>{diagnosis.details.map(item => <li key={item}>{item}</li>)}</ul></div><div><b>我的建议</b><ul>{diagnosis.suggestions.map(item => <li key={item}>{item}</li>)}</ul></div></div>
      <small>置信度：{diagnosis.confidence === "high" ? "高" : diagnosis.confidence === "medium" ? "中" : "低"} · 完全在本机分析</small>
    </section>}

    {history && <TrendChart history={history} range={historyRange} onRange={setHistoryRange} />}

    <div className="columns">
      <section className="card"><div className="section-title"><h3>正在盯着</h3><span>应用总占用 · 点击展开子进程</span></div>
        <div className="process-search-wrap"><span aria-hidden="true">⌕</span><input className="process-search" value={processQuery} onChange={event => setProcessQuery(event.target.value)} placeholder="搜索应用、进程或 PID" aria-label="搜索应用、进程或 PID" />{processQuery && <button onClick={() => setProcessQuery("")} aria-label="清除进程搜索">×</button>}</div>
        <div className="process-list">{visibleApplications.map(app => <div className="app-group" key={`${app.rootPid}-${app.name}`}>
          <button className="process application" onClick={() => setExpandedApp(expandedApp === app.rootPid ? null : app.rootPid)}>
            <span className="process-icon">{app.rootProcess.isCritical ? "🛡️" : expandedApp === app.rootPid ? "▾" : "▸"}</span><span className="process-name"><b>{app.name}</b><small>{app.memberCount} 个进程 · 主 PID {app.rootPid}</small></span>
            <span><b>{app.cpuPercent.toFixed(1)}%</b><small>{formatBytes(app.memoryBytes)}</small></span>
          </button>
          {expandedApp === app.rootPid && <div className="child-processes">{app.members.map(process => <button className="process child" key={`${process.pid}-${process.startedAt}`} onClick={() => inspect(process)}>
            <span className="process-icon">└</span><span className="process-name"><b>{process.pid === app.rootPid ? "主进程" : "子进程"}</b><small>PID {process.pid}{process.parentPid ? ` · 父 PID ${process.parentPid}` : ""} · {process.threadCount ?? 0} 线程</small></span><span><b>{process.cpuPercent.toFixed(1)}%</b><small>{formatBytes(process.memoryBytes)}</small></span>
          </button>)}</div>}
        </div>)}{!visibleApplications.length && <p className="empty">{normalizedProcessQuery ? "没有找到匹配的应用或进程。" : "还没有采集到应用数据。"}</p>}</div>
      </section>

      <section className="card"><div className="section-title"><h3>🐾 巡逻记录</h3><div className="timeline-title-actions"><span>最近事件</span>{status.timeline.length > 8 && <button onClick={() => setTimelineOpen(true)}>查看全部 {status.timeline.length} 条</button>}</div></div>
        <ol className="timeline">{status.timeline.slice(0, 8).map(item => <li key={item.id} className={`timeline-${item.kind}`}><time>{new Date(item.occurredAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</time><span>{item.message}</span></li>)}</ol>
      </section>
    </div>

    {message && <div className="toast" onClick={() => setMessage("")}>{message}</div>}
    {selectedHistoryMetric && <div className="modal-backdrop" onClick={() => setSelectedHistoryMetric(null)}><section className="modal report-modal metric-history-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">每个采样时间点</span><h3>{historyMetricLabel[selectedHistoryMetric]}历史明细</h3></div><button className="modal-close" onClick={() => setSelectedHistoryMetric(null)} aria-label="关闭历史明细">×</button></div>
      <div className="range-tabs metric-history-ranges">{[[10,"10 分钟"],[60,"1 小时"],[1440,"24 小时"],[10080,"7 天"]].map(([value,label]) => <button key={value} className={historyRange === value ? "active" : ""} onClick={() => setHistoryRange(Number(value))}>{label}</button>)}</div>
      <div className="report-scroll metric-history-scroll">{historyError ? <p className="history-error">历史数据读取失败：{historyError}</p> : history?.points.length ? <div className="metric-history-table" role="table">
        <div className="metric-history-row metric-history-head" role="row"><b>采样时间</b>{(["cpuPercent","memoryPercent","diskBps","networkBps"] as HistoryMetricKey[]).map(key => <b key={key} className={selectedHistoryMetric === key ? "selected" : ""}>{historyMetricLabel[key]}</b>)}</div>
        {[...history.points].reverse().map(point => <div className="metric-history-row" role="row" key={point.capturedAt}><time>{new Date(point.capturedAt).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" })}</time><span className={selectedHistoryMetric === "cpuPercent" ? "selected" : ""}>{point.cpuPercent.toFixed(1)}%</span><span className={selectedHistoryMetric === "memoryPercent" ? "selected" : ""}>{point.memoryPercent.toFixed(1)}%</span><span className={selectedHistoryMetric === "diskBps" ? "selected" : ""}>{formatRate(point.diskBps)}</span><span className={selectedHistoryMetric === "networkBps" ? "selected" : ""}>{formatRate(point.networkBps)}</span></div>)}
      </div> : <p className="empty">当前时间范围还没有历史采样数据。</p>}</div>
    </section></div>}
    {timelineOpen && <div className="modal-backdrop" onClick={() => setTimelineOpen(false)}><section className="modal report-modal timeline-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">本地保存的最近事件</span><h3>🐾 全部巡逻记录</h3></div><button className="modal-close" onClick={() => setTimelineOpen(false)} aria-label="关闭巡逻记录">×</button></div>
      <div className="report-scroll"><ol className="timeline timeline-full">{status.timeline.map(item => <li key={item.id} className={`timeline-${item.kind}`}><time>{new Date(item.occurredAt).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" })}</time><div><b>{timelineKindLabel[item.kind] ?? "事件"}</b><span>{item.message}</span></div></li>)}</ol>{!status.timeline.length && <p className="empty">还没有巡逻记录。</p>}</div>
    </section></div>}
    {hardwareOpen && snap && <div className="modal-backdrop" onClick={() => setHardwareOpen(false)}><section className="modal report-modal hardware-report" onClick={event => event.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">实时设备指标</span><h3>🖥️ 硬件监控</h3></div><button className="modal-close" onClick={() => setHardwareOpen(false)} aria-label="关闭硬件监控">×</button></div>
      <div className="report-scroll"><div className="report-tabs hardware-tabs" role="tablist">{([['cpu','CPU 核心'],['gpu','GPU'],['power','电池与传感器'],['disks','磁盘分区'],['network','网络适配器'],['apps','应用资源']] as const).map(([key,label]) => <button key={key} className={hardwareTab === key ? "active" : ""} onClick={() => setHardwareTab(key)}>{label}</button>)}</div>
      {hardwareTab === "cpu" && <div className="hardware-grid">{snap.hardware.cpuCores.map(core => <article key={core.name}><span>{core.name}</span><b>{core.usagePercent.toFixed(1)}%</b><i><em style={{width: `${core.usagePercent}%`}} /></i><small>{core.frequencyMhz} MHz</small></article>)}</div>}
      {hardwareTab === "gpu" && <div>{snap.hardware.gpus.map(gpu => <article className="hardware-row" key={gpu.name}><div><b>{gpu.name}</b><small>驱动实时指标</small></div><span>使用率 {gpu.usagePercent.toFixed(1)}%</span><span>显存 {formatBytes(gpu.memoryUsedBytes)} / {formatBytes(gpu.memoryTotalBytes)}</span></article>)}{!snap.hardware.gpus.length && <p className="availability">{snap.hardware.gpuStatus}</p>}</div>}
      {hardwareTab === "power" && <div className="sensor-section">{snap.hardware.battery ? <article className="hardware-row"><div><b>电池 {snap.hardware.battery.chargePercent}%</b><small>{snap.hardware.battery.charging ? "正在充电" : snap.hardware.battery.acConnected ? "已连接电源" : "正在放电"}</small></div><span>{snap.hardware.battery.lifeSeconds ? `预计 ${formatDuration(snap.hardware.battery.lifeSeconds)}` : "剩余时间未知"}</span></article> : <p className="availability">未检测到电池，台式机通常没有此项。</p>}<h4>温度</h4>{snap.hardware.temperatures.map(sensor => <article className="hardware-row" key={sensor.label}><b>{sensor.label}</b><span>{sensor.celsius.toFixed(1)}°C{sensor.maxCelsius ? ` · 峰值 ${sensor.maxCelsius.toFixed(1)}°C` : ""}</span></article>)}{!snap.hardware.temperatures.length && <p className="availability">硬件或驱动未向 Windows 提供温度传感器。</p>}<h4>风扇</h4>{snap.hardware.fans.map(fan => <article className="hardware-row" key={fan.label}><b>{fan.label}</b><span>{fan.rpm} RPM</span></article>)}{!snap.hardware.fans.length && <p className="availability">{snap.hardware.fanStatus}</p>}</div>}
      {hardwareTab === "disks" && <div>{snap.hardware.disks.map(disk => <article className="hardware-row" key={`${disk.name}-${disk.mountPoint}`}><div><b>{disk.name || "本地磁盘"} · {disk.mountPoint}</b><small>可用 {formatBytes(disk.availableBytes)} / {formatBytes(disk.totalBytes)}</small></div><span>读 {formatRate(disk.readBps)}</span><span>写 {formatRate(disk.writeBps)}</span></article>)}</div>}
      {hardwareTab === "network" && <div>{snap.hardware.networks.map(adapter => <article className="hardware-row" key={adapter.name}><b>{adapter.name}</b><span>下载 {formatRate(adapter.receivedBps)}</span><span>上传 {formatRate(adapter.transmittedBps)}</span></article>)}</div>}
      {hardwareTab === "apps" && <div><p className="availability">{snap.hardware.appNetworkStatus}</p>{snap.applications.map(app => <article className="hardware-row" key={`${app.rootPid}-${app.name}`}><div><b>{app.name}</b><small>{app.memberCount} 进程 · {app.members.reduce((sum,item) => sum + (item.handleCount ?? 0), 0)} 句柄</small></div><span>磁盘读 {formatRate(app.diskReadBps ?? 0)}</span><span>磁盘写 {formatRate(app.diskWriteBps ?? 0)}</span><span>网络 --</span></article>)}</div>}
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
          <span className={`risk-pill ${entry.riskLevel}`}>{entry.riskLevel === "medium" ? "需确认" : "正常"}</span><div><b>{entry.name}</b><code>{entry.command}</code><small>{entry.source}{entry.reasons.length ? ` · ${entry.reasons.join(" · ")}` : ""}</small></div>
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
        {usageSummary.topApps.length > 0 && <div className="usage-ranking"><h4>前台使用排行</h4>{usageSummary.topApps.slice(0, 5).map((app, index) => <div key={app.name}><span>{index + 1}. {app.name}</span><i><em style={{width: `${Math.max(4, app.foregroundSeconds / Math.max(1, usageSummary.topApps[0].foregroundSeconds) * 100)}%`}} /></i><b>{formatDuration(app.foregroundSeconds)}</b></div>)}</div>}
        {usageSummary.dailyUsage.length > 0 && <div className="daily-usage"><h4>每日使用与启动次数</h4><div>{usageSummary.dailyUsage.map(day => { const peak = Math.max(...usageSummary.dailyUsage.map(item => item.foregroundSeconds), 1); return <span key={day.date} title={`${day.date} · 前台 ${formatDuration(day.foregroundSeconds)} · 启动 ${day.launchCount} 次`}><i style={{height: `${Math.max(5, day.foregroundSeconds / peak * 100)}%`}}/><small>{day.date.slice(5)}</small><b>{day.launchCount} 次</b></span>})}</div></div>}
      </div>}
      {usageTab === "list" && <div className="usage-tab-panel usage-list-panel" role="tabpanel">
      <div className="usage-list-tools"><input className="usage-search" value={usageQuery} onChange={event => setUsageQuery(event.target.value)} placeholder="搜索应用名称" /><button onClick={exportUsage}>导出 CSV</button></div>
      <div className="usage-head"><span>应用</span><span>启动 / 关闭</span><span>运行时间</span><span>前台使用</span></div>
      <div className="usage-list">{visibleUsage.map(record => <article key={record.sessionId} className="usage-row">
        <div><b>{record.name}</b><small>PID {record.rootPid} · 峰值 {record.memberPeak} 个进程</small></div>
        <div><span>{new Date(record.startedAt).toLocaleString("zh-CN")}</span><small>{record.isRunning ? "仍在运行" : record.closedAt ? `关闭于 ${new Date(record.closedAt).toLocaleString("zh-CN")}` : "关闭时间未知"}</small></div>
        <div><b>{formatDuration(record.runtimeSeconds)}</b><small>后台 {formatDuration(record.backgroundSeconds)}</small></div>
        <div><b>{formatDuration(record.foregroundSeconds)}</b><small>{record.isRunning ? "● 活跃会话" : "已结束"}</small></div>
      </article>)}{!visibleUsage.length && <p className="empty">没有匹配的应用使用记录。</p>}</div>
      </div>}
      </div>
    </section></div>}
    {selected && <div className="modal-backdrop" onClick={() => setSelected(null)}><section className="modal process-actions-modal" onClick={e => e.stopPropagation()}>
      <span className="risk">进程操作</span><h3>{selected.name}</h3><p>PID {selected.pid} · CPU {selected.cpuPercent.toFixed(1)}% · {formatBytes(selected.memoryBytes)}</p>
      <div className="process-action-list"><button onClick={() => reveal(selected)}>📂 打开文件位置 <small>只读操作</small></button><button onClick={() => requestPriority(selected, "belowNormal")}>⬇ 调低优先级 <small>需要确认</small></button><button onClick={() => requestPriority(selected, "normal")}>↔ 恢复正常优先级 <small>需要确认</small></button><button className="danger-row" onClick={() => requestTerminate(selected)}>结束进程 <small>可能丢失未保存内容</small></button></div>
      <div className="actions"><button className="secondary" onClick={() => setSelected(null)}>取消</button></div>
    </section></div>}
    {settingsOpen && settings && <div className="modal-backdrop" onClick={() => setSettingsOpen(false)}><section className="modal settings-modal" onClick={e => e.stopPropagation()}>
      <span className="risk">设置中心</span><h3>巡逻方式</h3>
      <label>CPU 告警阈值 <output>{settings.cpuThreshold}%</output><input type="range" min="70" max="99" value={settings.cpuThreshold} onChange={e => setSettings({...settings, cpuThreshold: Number(e.target.value)})} /></label>
      <label>内存告警阈值 <output>{settings.memoryThreshold}%</output><input type="range" min="70" max="99" value={settings.memoryThreshold} onChange={e => setSettings({...settings, memoryThreshold: Number(e.target.value)})} /></label>
      <label>普通采样间隔 <select value={settings.samplingSeconds} onChange={e => setSettings({...settings, samplingSeconds: Number(e.target.value)})}><option value="2">2 秒</option><option value="5">5 秒</option><option value="10">10 秒</option><option value="30">30 秒</option></select></label>
      <label>历史保留天数 <select value={settings.retentionDays} onChange={e => setSettings({...settings, retentionDays: Number(e.target.value)})}><option value="1">1 天</option><option value="7">7 天</option><option value="30">30 天</option><option value="90">90 天</option></select></label>
      <label className="check"><input type="checkbox" checked={settings.lowPowerMode} onChange={e => setSettings({...settings, lowPowerMode: e.target.checked})} />低功耗模式（固定每 15 秒巡逻）</label>
      <label className="check"><input type="checkbox" checked={settings.notificationsEnabled} onChange={e => setSettings({...settings, notificationsEnabled: e.target.checked})} />Windows 异常通知</label>
      <label className="check"><input type="checkbox" checked={settings.autoStart} onChange={e => setSettings({...settings, autoStart: e.target.checked})} />登录 Windows 后自动在托盘巡逻</label>
      <button className="clear-memory" onClick={clearMemory}>清除所有本地记忆</button>
      <div className="actions"><button className="secondary" onClick={() => setSettingsOpen(false)}>取消</button><button className="primary" disabled={busy} onClick={persistSettings}>保存设置</button></div>
    </section></div>}
    {preview && <div className="modal-backdrop" onClick={() => setPreview(null)}><section className="modal" onClick={e => e.stopPropagation()}>
      <span className="risk">{preview.riskLevel} · 需要确认</span><h3>{preview.title}</h3><p>{preview.warning}</p>
      <div className="target"><b>{preview.target.name}</b><span>PID {preview.target.pid} · CPU {preview.target.cpuPercent.toFixed(1)}% · {formatBytes(preview.target.memoryBytes)}</span></div>
      {!preview.allowed && <p className="blocked">为了系统安全，大黄狗拒绝执行这个操作。</p>}
      <div className="actions"><button className="secondary" onClick={() => setPreview(null)}>先不处理</button>{preview.allowed && <button className={preview.action === "terminateProcess" ? "danger" : "primary"} disabled={busy} onClick={execute}>{busy ? "正在处理…" : preview.action === "terminateProcess" ? "确认结束进程" : "确认调整优先级"}</button>}</div>
    </section></div>}
  </main>;
}
