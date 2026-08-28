import { useEffect, useMemo, useRef, useState } from "react";
import { init, use } from "echarts/core";
import { SunburstChart, TreemapChart } from "echarts/charts";
import { TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { ECharts, EChartsCoreOption } from "echarts/core";
import type { HardwareSnapshot } from "./types";

use([TreemapChart, SunburstChart, TooltipComponent, CanvasRenderer]);
type DiskMetric = HardwareSnapshot["disks"][number];
type ChartMode = "treemap" | "sunburst";

function diskTitle(disk: DiskMetric): string {
  const mount = disk.mountPoint.trim();
  const name = disk.name.trim();
  return mount && name && mount.toLowerCase() !== name.toLowerCase() ? `${mount} ${name}` : mount || name || "本地磁盘";
}
function usagePercent(disk: DiskMetric): number {
  return disk.totalBytes <= 0 ? 0 : Math.max(0, Math.min(100, (disk.totalBytes - disk.availableBytes) / disk.totalBytes * 100));
}
function usedColor(percent: number): string {
  return percent >= 90 ? "#c5533f" : percent >= 75 ? "#d99c22" : "#d0a12d";
}
function formatBytes(value: number): string {
  const gib = value / 1024 / 1024 / 1024;
  return gib >= 1024 ? `${(gib / 1024).toFixed(1)} TB` : `${gib.toFixed(1)} GB`;
}
function formatRate(value: number): string {
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB/s`;
  if (value >= 1024) return `${(value / 1024).toFixed(0)} KB/s`;
  return `${value.toFixed(0)} B/s`;
}

function StorageChart({ disks, mode }: { disks: DiskMetric[]; mode: ChartMode }) {
  const host = useRef<HTMLDivElement | null>(null);
  const chart = useRef<ECharts | null>(null);
  const data = useMemo(() => disks.filter(disk => disk.totalBytes > 0).map(disk => {
    const percent = usagePercent(disk);
    return { name: diskTitle(disk), value: disk.totalBytes, children: [
      { name: "已用", value: Math.max(0, disk.totalBytes - disk.availableBytes), itemStyle: { color: usedColor(percent) } },
      { name: "可用", value: Math.max(0, disk.availableBytes), itemStyle: { color: "#e9e3d8" } }
    ] };
  }), [disks]);

  useEffect(() => {
    if (!host.current) return;
    chart.current = init(host.current, undefined, { renderer: "canvas" });
    const resize = new ResizeObserver(() => chart.current?.resize());
    resize.observe(host.current);
    return () => { resize.disconnect(); chart.current?.dispose(); chart.current = null; };
  }, []);

  useEffect(() => {
    const tooltip = { formatter: (params: { treePathInfo?: Array<{ name: string }>; name?: string; value?: number }) => {
      const path = params.treePathInfo?.map(item => item.name).filter(Boolean).join(" / ") || params.name || "存储";
      return `<b>${path}</b><br/>${formatBytes(Number(params.value ?? 0))}`;
    } };
    const option: EChartsCoreOption = mode === "treemap" ? {
      animationDuration: 350, tooltip,
      series: [{ type: "treemap", data, roam: false, nodeClick: false, breadcrumb: { show: false }, visibleMin: 1,
        label: { show: true, color: "#493f31", fontSize: 11 }, upperLabel: { show: true, height: 28, color: "#493f31", fontWeight: 700 },
        itemStyle: { borderColor: "#fffdf9", borderWidth: 3, gapWidth: 2 }, levels: [
          { itemStyle: { borderWidth: 0, gapWidth: 5 } },
          { color: ["#f1dfad", "#ead7a5", "#f3e6c5"], upperLabel: { show: true }, itemStyle: { borderColor: "#fffdf9", borderWidth: 3, gapWidth: 3 } },
          { label: { show: true, formatter: (p: { name?: string; value?: number }) => `${p.name}\n${formatBytes(Number(p.value ?? 0))}` }, itemStyle: { borderColor: "#fffdf9", borderWidth: 2 } }
        ] }]
    } : {
      animationDuration: 350, tooltip,
      series: [{ type: "sunburst", data, radius: ["18%", "92%"], sort: undefined, nodeClick: false,
        emphasis: { focus: "ancestor" }, label: { color: "#493f31", fontSize: 10, rotate: "radial", minAngle: 7 },
        itemStyle: { borderColor: "#fffdf9", borderWidth: 3 }, levels: [ {},
          { r0: "18%", r: "55%", label: { rotate: 0, fontWeight: 700 }, itemStyle: { color: "#f0dfae" } },
          { r0: "55%", r: "92%", label: { formatter: (p: { name?: string; value?: number }) => `${p.name}\n${formatBytes(Number(p.value ?? 0))}` } }
        ] }]
    };
    chart.current?.setOption(option, { notMerge: true });
  }, [data, mode]);
  return <div className="storage-chart" ref={host} aria-label={mode === "treemap" ? "磁盘空间矩形树图" : "磁盘空间旭日图"} />;
}

export default function StorageAnalysis({ disks }: { disks: DiskMetric[] }) {
  const [mode, setMode] = useState<ChartMode>("treemap");
  const total = disks.reduce((sum, disk) => sum + disk.totalBytes, 0);
  const available = disks.reduce((sum, disk) => sum + disk.availableBytes, 0);
  const used = Math.max(0, total - available);
  if (!disks.length) return <p className="empty">暂时没有读取到磁盘分区数据。</p>;
  return <>
    <div className="storage-overview">
      <article><span>分区数量</span><b>{disks.length}</b></article><article><span>总容量</span><b>{formatBytes(total)}</b></article>
      <article><span>已经使用</span><b>{formatBytes(used)}</b></article><article><span>剩余可用</span><b>{formatBytes(available)}</b></article>
    </div>
    <section className="storage-chart-card">
      <div className="storage-chart-head"><div><h4>磁盘空间占用</h4><small>按“分区 → 已用/可用”展示，悬停可查看容量</small></div>
        <div className="storage-chart-tabs" role="tablist" aria-label="存储图表类型">
          <button role="tab" aria-selected={mode === "treemap"} className={mode === "treemap" ? "active" : ""} onClick={() => setMode("treemap")}>矩形树图</button>
          <button role="tab" aria-selected={mode === "sunburst"} className={mode === "sunburst" ? "active" : ""} onClick={() => setMode("sunburst")}>旭日图</button>
        </div></div>
      <StorageChart disks={disks} mode={mode} />
    </section>
    <div className="storage-disk-list">{disks.map(disk => { const percent = usagePercent(disk); const diskUsed = Math.max(0, disk.totalBytes - disk.availableBytes); return <article key={`${disk.name}-${disk.mountPoint}`} className={percent >= 90 ? "critical" : percent >= 75 ? "warning" : ""}>
      <div><b>{diskTitle(disk)}</b><span>{percent.toFixed(1)}% 已用</span></div><i><em style={{ width: `${percent}%` }} /></i>
      <div className="storage-disk-values"><span>已用 {formatBytes(diskUsed)}</span><span>可用 {formatBytes(disk.availableBytes)}</span><span>总计 {formatBytes(disk.totalBytes)}</span></div>
      <small>实时读 {formatRate(disk.readBps)} · 写 {formatRate(disk.writeBps)}</small></article>; })}</div>
  </>;
}
