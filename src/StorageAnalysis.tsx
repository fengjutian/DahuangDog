import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { init, use } from "echarts/core";
import { SunburstChart, TreemapChart } from "echarts/charts";
import { TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { ECharts, EChartsCoreOption } from "echarts/core";
import { cancelStorageScan, listStorageDirectory, scanStorageTree } from "./api";
import type { HardwareSnapshot, StorageEntry, StorageScanResult } from "./types";

use([TreemapChart, SunburstChart, TooltipComponent, CanvasRenderer]);
type DiskMetric = HardwareSnapshot["disks"][number];
type ChartMode = "treemap" | "sunburst";
const scanCache = new Map<string, { savedAt: number; result: StorageScanResult }>();
const directoryCache = new Map<string, { savedAt: number; result: StorageScanResult }>();
const SCAN_CACHE_MS = 5 * 60 * 1000;

function diskTitle(disk: DiskMetric): string {
  const mount = disk.mountPoint.trim(), name = disk.name.trim();
  return mount && name && mount.toLowerCase() !== name.toLowerCase() ? `${mount} ${name}` : mount || name || "本地磁盘";
}
function formatBytes(value: number): string {
  if (value >= 1024 ** 4) return `${(value / 1024 ** 4).toFixed(1)} TB`;
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(1)} GB`;
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(1)} MB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${value} B`;
}
function chartNode(entry: StorageEntry): { name: string; value: number; path: string; kind: string; children?: ReturnType<typeof chartNode>[] } {
  return { name: entry.name, value: entry.kind === "directory" && entry.sizeBytes === 0 ? 1024 * 1024 : entry.sizeBytes, path: entry.path, kind: entry.kind,
    children: entry.children.length ? entry.children.map(chartNode) : undefined };
}

function StorageChart({ root, mode, onOpen }: { root: StorageEntry; mode: ChartMode; onOpen: (path: string) => void }) {
  const host = useRef<HTMLDivElement | null>(null);
  const chart = useRef<ECharts | null>(null);
  const data = useMemo(() => root.children.map(chartNode), [root]);
  useEffect(() => {
    if (!host.current) return;
    chart.current = init(host.current, undefined, { renderer: "canvas" });
    const resize = new ResizeObserver(() => chart.current?.resize()); resize.observe(host.current);
    return () => { resize.disconnect(); chart.current?.dispose(); chart.current = null; };
  }, []);
  useEffect(() => {
    const instance = chart.current;
    if (!instance) return;
    instance.off("click");
    instance.on("click", params => {
      const data = params.data as { kind?: string; path?: string } | undefined;
      if (data?.kind === "directory" && data.path) onOpen(data.path);
    });
    return () => { instance.off("click"); };
  }, [onOpen]);
  useEffect(() => {
    const tooltip = { formatter: (p: { data?: { path?: string; value?: number; kind?: string } }) =>
      `<b>${p.data?.path ?? ""}</b><br/>${p.data?.kind === "directory" ? "文件夹 · 点击打开" : p.data?.kind === "file" ? "文件" : "合并项目"}${p.data?.kind === "directory" && Number(p.data?.value) === 1024 * 1024 ? " · 大小待计算" : ` · ${formatBytes(Number(p.data?.value ?? 0))}`}` };
    const common = { data, nodeClick: false as const, emphasis: { focus: "ancestor" as const }, itemStyle: { borderColor: "#fffdf9", borderWidth: 2 } };
    const option: EChartsCoreOption = mode === "treemap" ? { animationDuration: 300, tooltip, series: [{
      ...common, type: "treemap", roam: true, breadcrumb: { show: true, bottom: 2, height: 22 }, leafDepth: 2,
      label: { show: true, color: "#493f31", formatter: (p: { name?: string; value?: number }) => `${p.name}\n${formatBytes(Number(p.value ?? 0))}` },
      upperLabel: { show: true, height: 25, color: "#493f31", fontWeight: 700 },
      levels: [{ itemStyle: { gapWidth: 4 } }, { colorSaturation: [.25, .55], itemStyle: { gapWidth: 2 } }, { colorSaturation: [.18, .45] }]
    }] } : { animationDuration: 300, tooltip, series: [{
      ...common, type: "sunburst", radius: ["12%", "92%"], sort: undefined,
      label: { color: "#493f31", fontSize: 9, minAngle: 5 },
      levels: [{}, { r0: "12%", r: "34%" }, { r0: "34%", r: "58%" }, { r0: "58%", r: "78%" }, { r0: "78%", r: "92%", label: { show: false } }]
    }] };
    chart.current?.setOption(option, { notMerge: true });
  }, [data, mode]);
  return <div className="storage-chart storage-tree-chart" ref={host} aria-label={mode === "treemap" ? "文件与文件夹矩形树图" : "文件与文件夹旭日图"} />;
}

export default function StorageAnalysis({ disks }: { disks: DiskMetric[] }) {
  const [mode, setMode] = useState<ChartMode>("treemap");
  const [selectedRoot, setSelectedRoot] = useState(disks[0]?.mountPoint ?? "");
  const [result, setResult] = useState<StorageScanResult | null>(null);
  const [scanning, setScanning] = useState(false);
  const [resultExact, setResultExact] = useState(false);
  const [error, setError] = useState("");
  const scanGeneration = useRef(0);
  const activeTask = useRef<string | null>(null);
  const cancelActiveScan = useCallback(() => {
    const taskId = activeTask.current;
    activeTask.current = null;
    if (taskId) void cancelStorageScan(taskId);
  }, []);
  useEffect(() => () => { cancelActiveScan(); }, [cancelActiveScan]);
  const browse = useCallback(async (path: string, force = false) => {
    cancelActiveScan();
    const cached = directoryCache.get(path);
    if (!force && cached && Date.now() - cached.savedAt < SCAN_CACHE_MS) {
      setSelectedRoot(path); setResult(cached.result); setResultExact(false); setError(""); setScanning(false); return;
    }
    const generation = ++scanGeneration.current;
    setSelectedRoot(path); setScanning(true); setError(""); setResult(null);
    try {
      const listed = await listStorageDirectory(path, force);
      if (generation === scanGeneration.current) { directoryCache.set(path, { savedAt: Date.now(), result: listed }); setResult(listed); setResultExact(false); }
    } catch (reason) { if (generation === scanGeneration.current) setError(String(reason)); }
    finally { if (generation === scanGeneration.current) setScanning(false); }
  }, [cancelActiveScan]);
  const parentPath = useMemo(() => {
    const normalized = selectedRoot.replace(/[\\/]+$/, "");
    const separator = Math.max(normalized.lastIndexOf("\\"), normalized.lastIndexOf("/"));
    if (separator < 0) return null;
    const parent = normalized.slice(0, separator);
    return /^[A-Za-z]:$/.test(parent) ? `${parent}\\` : parent || null;
  }, [selectedRoot]);
  const scan = async (root: string, force = false) => {
    const cached = scanCache.get(root);
    if (!force && cached && Date.now() - cached.savedAt < SCAN_CACHE_MS) {
      setSelectedRoot(root); setResult(cached.result); setResultExact(true); setError(""); return;
    }
    const generation = ++scanGeneration.current;
    cancelActiveScan();
    const taskId = crypto.randomUUID();
    activeTask.current = taskId;
    setSelectedRoot(root); setScanning(true); setError("");
    setResult({ root: { name: root, path: root, sizeBytes: 0, kind: "directory", children: [] }, fileCount: 0, directoryCount: 0, skippedCount: 0, cacheHit: false, indexedAt: Date.now() });
    let pending: StorageEntry[] = [];
    let flushTimer: number | undefined;
    const flush = () => {
      flushTimer = undefined;
      if (generation !== scanGeneration.current || !pending.length) return;
      const batch = pending; pending = [];
      setResult(current => {
        if (!current || current.root.path !== root) return current;
        const children = [...current.root.children, ...batch].sort((a, b) => b.sizeBytes - a.sizeBytes);
        return { ...current, root: { ...current.root, children, sizeBytes: children.reduce((sum, item) => sum + item.sizeBytes, 0) } };
      });
    };
    try {
      const completed = await scanStorageTree(root, taskId, force, entry => {
        if (generation !== scanGeneration.current) return;
        pending.push(entry);
        flushTimer ??= window.setTimeout(flush, 150);
      });
      if (flushTimer != null) window.clearTimeout(flushTimer);
      if (generation === scanGeneration.current) { scanCache.set(root, { savedAt: Date.now(), result: completed }); setResult(completed); setResultExact(true); }
    } catch (reason) { if (generation === scanGeneration.current && !String(reason).includes("扫描已取消")) setError(String(reason)); }
    finally {
      if (activeTask.current === taskId) activeTask.current = null;
      if (generation === scanGeneration.current) setScanning(false);
    }
  };
  if (!disks.length) return <p className="empty">暂时没有读取到磁盘分区数据。</p>;
  return <>
    <div className="storage-drive-picker" role="tablist" aria-label="选择要扫描的磁盘">
      {disks.map(disk => <button key={disk.mountPoint} className={selectedRoot.toLowerCase().startsWith(disk.mountPoint.toLowerCase()) ? "active" : ""} disabled={scanning} onClick={() => void browse(disk.mountPoint)}>
        <b>{diskTitle(disk)}</b><span>立即浏览</span>
      </button>)}
    </div>
    {!result && !scanning && !error && <div className="storage-scan-prompt"><b>选择一个磁盘开始分析</b><p>默认只读取当前一层；点击文件夹继续深入，需要精确占比时再计算当前目录。</p></div>}
    {scanning && !result?.root.children.length && <div className="storage-scan-prompt scanning"><span className="storage-spinner"/><b>正在读取 {selectedRoot}</b><p>按需模式只枚举当前目录，不会扫描整个磁盘。</p>{activeTask.current && <button onClick={cancelActiveScan}>取消扫描</button>}</div>}
    {error && <div className="storage-scan-prompt error"><b>读取失败</b><p>{error}</p><button onClick={() => void browse(selectedRoot, true)}>重新读取</button></div>}
    {result && (!scanning || result.root.children.length > 0) && <>
      {scanning && <div className="storage-progress-note"><span className="storage-spinner"/><span>正在逐项计算，已显示 {result.root.children.length} 个顶层项目；图表可立即使用。</span></div>}
      {result.cacheHit && !scanning && <div className="storage-progress-note"><span>已复用 SQLite 目录索引；目录修改后会自动重新解析。</span></div>}
      <div className="storage-overview storage-scan-overview">
        <article><span>{resultExact ? "已统计容量" : "当前层文件大小"}</span><b>{formatBytes(result.root.sizeBytes)}</b></article><article><span>{resultExact ? "全部文件" : "当前层文件"}</span><b>{result.fileCount.toLocaleString()}</b></article>
        <article><span>{resultExact ? "全部文件夹" : "当前层文件夹"}</span><b>{result.directoryCount.toLocaleString()}</b></article><article><span>无权限/已跳过</span><b>{result.skippedCount.toLocaleString()}</b></article>
      </div>
      <section className="storage-chart-card"><div className="storage-chart-head"><div><h4>{selectedRoot} 文件占用</h4><small>文件大小立即显示；文件夹点击进入，精确占比需单独计算</small></div>
        <div className="storage-chart-actions">{parentPath && <button className="storage-rescan" disabled={scanning} onClick={() => void browse(parentPath)}>返回上级</button>}<button className="storage-rescan" disabled={scanning} onClick={() => void scan(selectedRoot)}>精确计算当前目录</button>{scanning && <button className="storage-rescan" onClick={cancelActiveScan}>取消扫描</button>}<button className="storage-rescan" disabled={scanning} onClick={() => void browse(selectedRoot, true)}>刷新</button><div className="storage-chart-tabs" role="tablist"><button className={mode === "treemap" ? "active" : ""} onClick={() => setMode("treemap")}>矩形树图</button><button className={mode === "sunburst" ? "active" : ""} onClick={() => setMode("sunburst")}>旭日图</button></div></div>
      </div><StorageChart root={result.root} mode={mode} onOpen={browse} /></section>
    </>}
  </>;
}
