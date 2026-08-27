import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { clearLocalMemory, confirmAction, diagnosePerformance, getAppUsageHistory, getAppUsageSummary, getCurrentStatus, getHistory, getSecurityReport, getSettings, openProcessLocation, preparePriority, prepareTerminate, saveSettings } from "./api";
import type { ActionPreview, AppUsageRecord, AppUsageSummary, CurrentStatus, HistorySummary, LocalDiagnosis, MetricPoint, ProcessSample, SecurityReport, UserSettings } from "./types";

const stateLabel: Record<string, string> = {
  idle: "在狗窝待命", patrol: "正在巡逻", suspicious: "竖起耳朵",
  investigating: "正在调查", awaitingApproval: "等你决定",
  verifying: "正在确认效果", resolved: "问题解决"
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

function Metric({ label, value, tone }: { label: string; value: string; tone?: "warn" }) {
  return <div className={`metric ${tone ?? ""}`}><span>{label}</span><strong>{value}</strong></div>;
}

function linePath(points: MetricPoint[], key: "cpuPercent" | "memoryPercent"): string {
  if (points.length < 2) return "";
  return points.map((point, index) => {
    const x = index / (points.length - 1) * 100;
    const y = 100 - Math.max(0, Math.min(100, point[key]));
    return `${index ? "L" : "M"}${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
}

function TrendChart({ history }: { history: HistorySummary }) {
  return <section className="card trend-card">
    <div className="section-title"><h3>最近趋势</h3><span>最近 {history.points.length * 2} 秒</span></div>
    <svg className="trend" viewBox="0 0 100 100" preserveAspectRatio="none" aria-label="CPU 和内存趋势图">
      <path className="grid-line" d="M0,25 H100 M0,50 H100 M0,75 H100" />
      <path className="memory-line" d={linePath(history.points, "memoryPercent")} />
      <path className="cpu-line" d={linePath(history.points, "cpuPercent")} />
    </svg>
    <div className="legend"><span className="cpu-dot">CPU · 基线 {history.baselineCpuPercent.toFixed(0)}%</span><span className="memory-dot">内存 · 基线 {history.baselineMemoryPercent.toFixed(0)}%</span></div>
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
  const [diagnosis, setDiagnosis] = useState<LocalDiagnosis | null>(null);
  const [security, setSecurity] = useState<SecurityReport | null>(null);
  const [selected, setSelected] = useState<ProcessSample | null>(null);
  const [settings, setSettings] = useState<UserSettings | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [expandedApp, setExpandedApp] = useState<number | null>(null);
  const [usage, setUsage] = useState<AppUsageRecord[] | null>(null);
  const [usageSummary, setUsageSummary] = useState<AppUsageSummary | null>(null);
  const [usageQuery, setUsageQuery] = useState("");
  const [securityFilter, setSecurityFilter] = useState<"all" | "medium" | "low">("all");

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
    const load = () => void getHistory().then(setHistory).catch(() => undefined);
    load();
    const timer = window.setInterval(load, 10_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => { void getSettings().then(setSettings).catch(() => undefined); }, []);

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
      const [records, summary] = await Promise.all([getAppUsageHistory(), getAppUsageSummary(7)]);
      setUsage(records);
      setUsageSummary(summary);
    }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }

  if (!status) return <main className="loading">🐕 大黄狗正在醒来……</main>;
  const snap = status.snapshot;
  const visibleUsage = usage?.filter(record => record.name.toLowerCase().includes(usageQuery.trim().toLowerCase())) ?? [];
  const visiblePrograms = security?.programs.filter(program => securityFilter === "all" || program.riskLevel === securityFilter) ?? [];

  return <main className="shell">
    <header><div className="brand"><span className="dog">🐕</span><div><h1>大黄狗</h1><p>住在 Windows 里的 AI 看门狗</p></div></div><div className="header-actions"><button onClick={showUsage} disabled={busy}>⏱ 使用记录</button><button onClick={() => setSettingsOpen(true)}>⚙️ 设置</button><button onClick={scanSecurity} disabled={busy}>🛡️ 看门报告</button><span className="live"><i />{stateLabel[status.dogState]}</span></div></header>

    <section className="hero">
      <div className="avatar" aria-hidden="true">🐕</div>
      <div><span className="eyebrow">今天的巡逻报告</span><h2>{status.summary}</h2><p>健康度 <b>{status.healthScore}</b> / 100</p><button className="diagnose-button" onClick={diagnose} disabled={busy}>{busy ? "正在检查…" : "大黄，电脑为什么卡？"}</button></div>
    </section>

    <section className="metrics">
      <Metric label="CPU" value={snap ? `${snap.cpuPercent.toFixed(0)}%` : "--"} tone={snap && snap.cpuPercent >= 90 ? "warn" : undefined} />
      <Metric label="内存" value={snap ? `${snap.memoryPercent.toFixed(0)}%` : "--"} tone={snap && snap.memoryPercent >= 90 ? "warn" : undefined} />
      <Metric label="已用内存" value={snap ? formatBytes(snap.usedMemoryBytes) : "--"} />
      <Metric label="磁盘读 / 写" value={snap ? `${formatRate(snap.diskReadBps)} / ${formatRate(snap.diskWriteBps)}` : "--"} />
      <Metric label="网络下 / 上" value={snap ? `${formatRate(snap.networkReceiveBps)} / ${formatRate(snap.networkSendBps)}` : "--"} />
      <Metric label="发现" value={`${status.findings.length} 个`} tone={status.findings.length ? "warn" : undefined} />
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

    {history && <TrendChart history={history} />}

    <div className="columns">
      <section className="card"><div className="section-title"><h3>正在盯着</h3><span>应用总占用 · 点击展开子进程</span></div>
        <div className="process-list">{snap?.applications.slice(0, 8).map(app => <div className="app-group" key={`${app.rootPid}-${app.name}`}>
          <button className="process application" onClick={() => setExpandedApp(expandedApp === app.rootPid ? null : app.rootPid)}>
            <span className="process-icon">{app.rootProcess.isCritical ? "🛡️" : expandedApp === app.rootPid ? "▾" : "▸"}</span><span className="process-name"><b>{app.name}</b><small>{app.memberCount} 个进程 · 主 PID {app.rootPid}</small></span>
            <span><b>{app.cpuPercent.toFixed(1)}%</b><small>{formatBytes(app.memoryBytes)}</small></span>
          </button>
          {expandedApp === app.rootPid && <div className="child-processes">{app.members.map(process => <button className="process child" key={`${process.pid}-${process.startedAt}`} onClick={() => inspect(process)}>
            <span className="process-icon">└</span><span className="process-name"><b>{process.pid === app.rootPid ? "主进程" : "子进程"}</b><small>PID {process.pid}{process.parentPid ? ` · 父 PID ${process.parentPid}` : ""}</small></span><span><b>{process.cpuPercent.toFixed(1)}%</b><small>{formatBytes(process.memoryBytes)}</small></span>
          </button>)}</div>}
        </div>)}{!snap?.applications.length && <p className="empty">还没有采集到应用数据。</p>}</div>
      </section>

      <section className="card"><div className="section-title"><h3>🐾 巡逻记录</h3><span>最近事件</span></div>
        <ol className="timeline">{status.timeline.slice(0, 8).map(item => <li key={item.id}><time>{new Date(item.occurredAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</time><span>{item.message}</span></li>)}</ol>
      </section>
    </div>

    {message && <div className="toast" onClick={() => setMessage("")}>{message}</div>}
    {security && <div className="modal-backdrop" onClick={() => setSecurity(null)}><section className="modal report-modal security-report" onClick={e => e.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">只读安全扫描</span><h3>🛡️ 看门报告</h3></div><button className="modal-close" onClick={() => setSecurity(null)} aria-label="关闭看门报告">×</button></div>
      <div className="report-scroll">
        <div className="security-scoreboard">
          <div className={`security-score ${security.securityScore < 70 ? "attention" : ""}`}><strong>{security.securityScore}</strong><span>安全分</span></div>
          <div><b>{security.mediumRiskCount}</b><span>需要确认</span></div>
          <div><b>{security.lowRiskCount}</b><span>低风险信号</span></div>
          <div><b>{security.signedPrograms}/{security.scannedPrograms}</b><span>签名有效</span></div>
        </div>
        <div className="security-summary"><b>{security.summary}</b><span>扫描于 {new Date(security.scannedAt).toLocaleString("zh-CN")} · {security.startupEntries.length} 个启动项</span></div>
        <p className="security-note">未验证不等于恶意程序。大黄狗只展示客观信号，请结合来源和用途判断。</p>
        <div className="report-filters"><button className={securityFilter === "all" ? "active" : ""} onClick={() => setSecurityFilter("all")}>全部</button><button className={securityFilter === "medium" ? "active" : ""} onClick={() => setSecurityFilter("medium")}>需要确认</button><button className={securityFilter === "low" ? "active" : ""} onClick={() => setSecurityFilter("low")}>低风险</button></div>
        {visiblePrograms.length > 0 && <div className="security-group"><h4>运行中的程序</h4>{visiblePrograms.map(program => <article className="security-item" key={`${program.pid}-${program.path}`}>
          <span className={`risk-pill ${program.riskLevel}`}>{program.riskLevel === "medium" ? "需确认" : "低风险"}</span><div><b>{program.name}</b><code>{program.path}</code><small>{program.reasons.length ? program.reasons.join(" · ") : "未发现额外风险信号"}</small></div><span>{program.signatureStatus === "valid" ? "✓ 签名有效" : "? 签名未验证"}</span>
        </article>)}</div>}
        <div className="security-group"><h4>开机启动项</h4>{security.startupEntries.length ? security.startupEntries.map(entry => <article className="security-item startup" key={`${entry.source}-${entry.name}`}>
          <span className={`risk-pill ${entry.riskLevel}`}>{entry.riskLevel === "medium" ? "需确认" : "正常"}</span><div><b>{entry.name}</b><code>{entry.command}</code><small>{entry.source}{entry.reasons.length ? ` · ${entry.reasons.join(" · ")}` : ""}</small></div>
        </article>) : <p className="empty">没有读取到常见启动项。</p>}</div>
      </div>
    </section></div>}
    {usage && <div className="modal-backdrop" onClick={() => setUsage(null)}><section className="modal report-modal usage-report" onClick={e => e.stopPropagation()}>
      <div className="section-title"><div><span className="eyebrow">应用生命周期</span><h3>⏱ 使用记录</h3></div><button className="modal-close" onClick={() => setUsage(null)} aria-label="关闭使用记录">×</button></div>
      <p className="security-note">启动与运行时间来自进程生命周期；前台使用时间从大黄狗首次观察后累计。</p>
      {usageSummary && <>
        <div className="usage-summary">
          <div><b>{formatDuration(usageSummary.totalForegroundSeconds)}</b><span>近 7 天前台使用</span></div>
          <div><b>{formatDuration(usageSummary.totalBackgroundSeconds)}</b><span>后台运行</span></div>
          <div><b>{usageSummary.applicationCount}</b><span>使用过的应用</span></div>
          <div><b>{usageSummary.longestUsedApp ?? "暂无"}</b><span>最常使用</span></div>
        </div>
        {usageSummary.topApps.length > 0 && <div className="usage-ranking"><h4>前台使用排行</h4>{usageSummary.topApps.slice(0, 5).map((app, index) => <div key={app.name}><span>{index + 1}. {app.name}</span><i><em style={{width: `${Math.max(4, app.foregroundSeconds / Math.max(1, usageSummary.topApps[0].foregroundSeconds) * 100)}%`}} /></i><b>{formatDuration(app.foregroundSeconds)}</b></div>)}</div>}
      </>}
      <input className="usage-search" value={usageQuery} onChange={event => setUsageQuery(event.target.value)} placeholder="搜索应用名称" />
      <div className="usage-head"><span>应用</span><span>启动 / 关闭</span><span>运行时间</span><span>前台使用</span></div>
      <div className="usage-list report-scroll">{visibleUsage.map(record => <article key={record.sessionId} className="usage-row">
        <div><b>{record.name}</b><small>PID {record.rootPid} · 峰值 {record.memberPeak} 个进程</small></div>
        <div><span>{new Date(record.startedAt).toLocaleString("zh-CN")}</span><small>{record.isRunning ? "仍在运行" : record.closedAt ? `关闭于 ${new Date(record.closedAt).toLocaleString("zh-CN")}` : "关闭时间未知"}</small></div>
        <div><b>{formatDuration(record.runtimeSeconds)}</b><small>后台 {formatDuration(record.backgroundSeconds)}</small></div>
        <div><b>{formatDuration(record.foregroundSeconds)}</b><small>{record.isRunning ? "● 活跃会话" : "已结束"}</small></div>
      </article>)}{!visibleUsage.length && <p className="empty">没有匹配的应用使用记录。</p>}</div>
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
