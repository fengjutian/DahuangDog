import { useEffect, useMemo, useRef, useState } from "react";
import { init, use } from "echarts/core";
import { SunburstChart, TreemapChart } from "echarts/charts";
import { TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { ECharts, EChartsCoreOption } from "echarts/core";
import { scanStorageTree } from "./api";
import type { HardwareSnapshot, StorageEntry, StorageScanResult } from "./types";

use([TreemapChart, SunburstChart, TooltipComponent, CanvasRenderer]);
type DiskMetric = HardwareSnapshot["disks"][number];
type ChartMode = "treemap" | "sunburst";
const scanCache = new Map<string, { savedAt: number; result: StorageScanResult }>();
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
  return { name: entry.name, value: entry.sizeBytes, path: entry.path, kind: entry.kind,
    children: entry.children.length ? entry.children.map(chartNode) : undefined };
}

function StorageChart({ root, mode }: { root: StorageEntry; mode: ChartMode }) {
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
    const tooltip = { formatter: (p: { data?: { path?: string; value?: number; kind?: string } }) =>
      `<b>${p.data?.path ?? ""}</b><br/>${p.data?.kind === "directory" ? "文件夹" : p.data?.kind === "file" ? "文件" : "合并项目"} · ${formatBytes(Number(p.data?.value ?? 0))}` };
    const common = { data, nodeClick: "zoomToNode" as const, emphasis: { focus: "ancestor" as const }, itemStyle: { borderColor: "#fffdf9", borderWidth: 2 } };
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
  const [error, setError] = useState("");
  const scan = async (root: string, force = false) => {
    const cached = scanCache.get(root);
    if (!force && cached && Date.now() - cached.savedAt < SCAN_CACHE_MS) {
      setSelectedRoot(root); setResult(cached.result); setError(""); return;
    }
    setSelectedRoot(root); setScanning(true); setError("");
    setResult({ root: { name: root, path: root, sizeBytes: 0, kind: "directory", children: [] }, fileCount: 0, directoryCount: 0, skippedCount: 0 });
    try { const completed = await scanStorageTree(root, entry => setResult(current => {
      if (!current || current.root.path !== root) return current;
      const children = [...current.root.children, entry].sort((a, b) => b.sizeBytes - a.sizeBytes);
      return { ...current, root: { ...current.root, children, sizeBytes: children.reduce((sum, item) => sum + item.sizeBytes, 0) } };
    })); scanCache.set(root, { savedAt: Date.now(), result: completed }); setResult(completed); }
    catch (reason) { setError(String(reason)); }
    finally { setScanning(false); }
  };
  if (!disks.length) return <p className="empty">暂时没有读取到磁盘分区数据。</p>;
  return <>
    <div className="storage-drive-picker" role="tablist" aria-label="选择要扫描的磁盘">
      {disks.map(disk => <button key={disk.mountPoint} className={selectedRoot === disk.mountPoint ? "active" : ""} disabled={scanning} onClick={() => void scan(disk.mountPoint)}>
        <b>{diskTitle(disk)}</b><span>扫描文件与文件夹</span>
      </button>)}
    </div>
    {!result && !scanning && !error && <div className="storage-scan-prompt"><b>选择一个磁盘开始分析</b><p>将递归读取文件与文件夹的大小。扫描时间取决于文件数量，受保护的项目会自动跳过。</p></div>}
    {scanning && !result?.root.children.length && <div className="storage-scan-prompt scanning"><span className="storage-spinner"/><b>正在读取 {selectedRoot} 第一层</b><p>每完成一个文件或文件夹就会立即显示，不必等待整个磁盘完成。</p></div>}
    {error && <div className="storage-scan-prompt error"><b>扫描失败</b><p>{error}</p><button onClick={() => void scan(selectedRoot)}>重新扫描</button></div>}
    {result && (!scanning || result.root.children.length > 0) && <>
      {scanning && <div className="storage-progress-note"><span className="storage-spinner"/><span>正在逐项计算，已显示 {result.root.children.length} 个顶层项目；图表可立即使用。</span></div>}
      <div className="storage-overview storage-scan-overview">
        <article><span>已统计容量</span><b>{formatBytes(result.root.sizeBytes)}</b></article><article><span>文件</span><b>{result.fileCount.toLocaleString()}</b></article>
        <article><span>文件夹</span><b>{result.directoryCount.toLocaleString()}</b></article><article><span>无权限/已跳过</span><b>{result.skippedCount.toLocaleString()}</b></article>
      </div>
      <section className="storage-chart-card"><div className="storage-chart-head"><div><h4>{selectedRoot} 文件占用</h4><small>单击区域可下钻，点击底部路径可返回上级</small></div>
        <div className="storage-chart-actions"><button className="storage-rescan" disabled={scanning} onClick={() => void scan(selectedRoot, true)}>重新扫描</button><div className="storage-chart-tabs" role="tablist"><button className={mode === "treemap" ? "active" : ""} onClick={() => setMode("treemap")}>矩形树图</button><button className={mode === "sunburst" ? "active" : ""} onClick={() => setMode("sunburst")}>旭日图</button></div></div>
      </div><StorageChart root={result.root} mode={mode} /></section>
    </>}
  </>;
}
