import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { confirmAction, getCurrentStatus, prepareTerminate } from "./api";
import type { ActionPreview, CurrentStatus, ProcessSample } from "./types";

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

function Metric({ label, value, tone }: { label: string; value: string; tone?: "warn" }) {
  return <div className={`metric ${tone ?? ""}`}><span>{label}</span><strong>{value}</strong></div>;
}

export default function App() {
  const [status, setStatus] = useState<CurrentStatus | null>(null);
  const [preview, setPreview] = useState<ActionPreview | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);

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

  async function inspect(process: ProcessSample) {
    try { setPreview(await prepareTerminate(process.pid, process.startedAt)); }
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

  if (!status) return <main className="loading">🐕 大黄狗正在醒来……</main>;
  const snap = status.snapshot;

  return <main className="shell">
    <header><div className="brand"><span className="dog">🐕</span><div><h1>大黄狗</h1><p>住在 Windows 里的 AI 看门狗</p></div></div><span className="live"><i />{stateLabel[status.dogState]}</span></header>

    <section className="hero">
      <div className="avatar" aria-hidden="true">🐕</div>
      <div><span className="eyebrow">今天的巡逻报告</span><h2>{status.summary}</h2><p>健康度 <b>{status.healthScore}</b> / 100</p></div>
    </section>

    <section className="metrics">
      <Metric label="CPU" value={snap ? `${snap.cpuPercent.toFixed(0)}%` : "--"} tone={snap && snap.cpuPercent >= 90 ? "warn" : undefined} />
      <Metric label="内存" value={snap ? `${snap.memoryPercent.toFixed(0)}%` : "--"} tone={snap && snap.memoryPercent >= 90 ? "warn" : undefined} />
      <Metric label="已用内存" value={snap ? formatBytes(snap.usedMemoryBytes) : "--"} />
      <Metric label="发现" value={`${status.findings.length} 个`} tone={status.findings.length ? "warn" : undefined} />
    </section>

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
    {preview && <div className="modal-backdrop" onClick={() => setPreview(null)}><section className="modal" onClick={e => e.stopPropagation()}>
      <span className="risk">{preview.riskLevel} · 需要确认</span><h3>{preview.title}</h3><p>{preview.warning}</p>
      <div className="target"><b>{preview.target.name}</b><span>PID {preview.target.pid} · CPU {preview.target.cpuPercent.toFixed(1)}% · {formatBytes(preview.target.memoryBytes)}</span></div>
      {!preview.allowed && <p className="blocked">为了系统安全，大黄狗拒绝执行这个操作。</p>}
      <div className="actions"><button className="secondary" onClick={() => setPreview(null)}>先不处理</button>{preview.allowed && <button className="danger" disabled={busy} onClick={execute}>{busy ? "正在处理…" : "确认结束进程"}</button>}</div>
    </section></div>}
  </main>;
}
