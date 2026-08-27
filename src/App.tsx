import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { clearLocalMemory, confirmAction, diagnosePerformance, getCurrentStatus, getHistory, getSecurityReport, getSettings, openProcessLocation, preparePriority, prepareTerminate, saveSettings } from "./api";
import type { ActionPreview, CurrentStatus, HistorySummary, LocalDiagnosis, MetricPoint, ProcessSample, SecurityReport, UserSettings } from "./types";

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

  if (!status) return <main className="loading">🐕 大黄狗正在醒来……</main>;
  const snap = status.snapshot;

  return <main className="shell">
    <header><div className="brand"><span className="dog">🐕</span><div><h1>大黄狗</h1><p>住在 Windows 里的 AI 看门狗</p></div></div><div className="header-actions"><button onClick={() => setSettingsOpen(true)}>⚙️ 设置</button><button onClick={scanSecurity} disabled={busy}>🛡️ 看门报告</button><span className="live"><i />{stateLabel[status.dogState]}</span></div></header>

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

    {security && <section className="security-report card">
      <div className="section-title"><div><span className="eyebrow">只读安全扫描</span><h3>🛡️ 看门报告</h3></div><button onClick={() => setSecurity(null)}>收起</button></div>
      <div className="security-summary"><b>{security.summary}</b><span>扫描 {security.scannedPrograms} 个程序 · {security.signedPrograms} 个签名有效 · {security.startupEntries.length} 个启动项</span></div>
      <p className="security-note">未验证不等于恶意程序。大黄狗只展示客观信号，请结合来源和用途判断。</p>
      {security.programs.length > 0 && <div className="security-group"><h4>运行中的程序</h4>{security.programs.map(program => <article className="security-item" key={`${program.pid}-${program.path}`}>
        <span className={`risk-pill ${program.riskLevel}`}>{program.riskLevel === "medium" ? "需确认" : "低风险"}</span><div><b>{program.name}</b><code>{program.path}</code><small>{program.reasons.length ? program.reasons.join(" · ") : "未发现额外风险信号"}</small></div><span>{program.signatureStatus === "valid" ? "✓ 签名有效" : "? 签名未验证"}</span>
      </article>)}</div>}
      <div className="security-group"><h4>开机启动项</h4>{security.startupEntries.length ? security.startupEntries.map(entry => <article className="security-item startup" key={`${entry.source}-${entry.name}`}>
        <span className={`risk-pill ${entry.riskLevel}`}>{entry.riskLevel === "medium" ? "需确认" : "正常"}</span><div><b>{entry.name}</b><code>{entry.command}</code><small>{entry.source}{entry.reasons.length ? ` · ${entry.reasons.join(" · ")}` : ""}</small></div>
      </article>) : <p className="empty">没有读取到常见启动项。</p>}</div>
    </section>}

    {history && <TrendChart history={history} />}

    <div className="columns">
      <section className="card"><div className="section-title"><h3>正在盯着</h3><span>资源占用靠前</span></div>
        <div className="process-list">{snap?.processes.slice(0, 6).map(process =>
          <button className="process" key={`${process.pid}-${process.startedAt}`} onClick={() => inspect(process)}>
            <span className="process-icon">{process.isCritical ? "🛡️" : "●"}</span><span className="process-name"><b>{process.name}</b><small>PID {process.pid}</small></span>
            <span><b>{process.cpuPercent.toFixed(1)}%</b><small>{formatBytes(process.memoryBytes)}</small></span>
          </button>)}{!snap?.processes.length && <p className="empty">还没有采集到进程数据。</p>}</div>
      </section>

      <section className="card"><div className="section-title"><h3>🐾 巡逻记录</h3><span>最近事件</span></div>
        <ol className="timeline">{status.timeline.slice(0, 8).map(item => <li key={item.id}><time>{new Date(item.occurredAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</time><span>{item.message}</span></li>)}</ol>
      </section>
    </div>

    {message && <div className="toast" onClick={() => setMessage("")}>{message}</div>}
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
