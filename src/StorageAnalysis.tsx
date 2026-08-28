import { useEffect, useRef } from "react";
import { init, use } from "echarts/core";
import { BarChart } from "echarts/charts";
import { GridComponent, LegendComponent, TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { ECharts, EChartsCoreOption } from "echarts/core";
import type { HardwareSnapshot } from "./types";

use([BarChart, GridComponent, LegendComponent, TooltipComponent, CanvasRenderer]);

type DiskMetric = HardwareSnapshot["disks"][number];

function diskTitle(disk: DiskMetric): string {
  const mount = disk.mountPoint.trim();
  const name = disk.name.trim();
  if (mount && name && mount.toLowerCase() !== name.toLowerCase()) return `${mount}  ${name}`;
  return mount || name || "本地磁盘";
}

function usagePercent(disk: DiskMetric): number {
  if (disk.totalBytes <= 0) return 0;
  return Math.max(0, Math.min(100, (disk.totalBytes - disk.availableBytes) / disk.totalBytes * 100));
}

function usedColor(percent: number): string {
  if (percent >= 90) return "#c5533f";
  if (percent >= 75) return "#d99c22";
  return "#d0a12d";
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

function StorageChart({ disks }: { disks: DiskMetric[] }) {
  const host = useRef<HTMLDivElement | null>(null);
  const chart = useRef<ECharts | null>(null);

  useEffect(() => {
    if (!host.current) return;
    chart.current = init(host.current, undefined, { renderer: "canvas" });
    const resize = new ResizeObserver(() => chart.current?.resize());
    resize.observe(host.current);
    return () => {
      resize.disconnect();
      chart.current?.dispose();
      chart.current = null;
    };
  }, []);

  useEffect(() => {
    const labels = disks.map(diskTitle);
    const used = disks.map(disk => usagePercent(disk));
    const available = used.map(percent => 100 - percent);
    const option: EChartsCoreOption = {
      animationDuration: 350,
      color: ["#d0a12d", "#e9e3d8"],
      tooltip: {
        trigger: "axis",
        axisPointer: { type: "shadow" },
        valueFormatter: (value: unknown) => `${Number(value).toFixed(1)}%`
      },
      legend: { top: 0, right: 4, itemWidth: 12, itemHeight: 8, textStyle: { color: "#756957", fontSize: 11 } },
      grid: { top: 42, right: 24, bottom: 38, left: 112, containLabel: false },
      xAxis: {
        type: "value",
        min: 0,
        max: 100,
        name: "占用率",
        nameLocation: "middle",
        nameGap: 26,
        axisLabel: { formatter: "{value}%", color: "#8f836f", fontSize: 10 },
        axisLine: { show: true, lineStyle: { color: "#d9cfbf" } },
        splitLine: { lineStyle: { color: "#eee7dc" } }
      },
      yAxis: {
        type: "category",
        data: labels,
        axisTick: { show: false },
        axisLine: { lineStyle: { color: "#d9cfbf" } },
        axisLabel: { color: "#5c5141", fontSize: 11, width: 96, overflow: "truncate" }
      },
      series: [
        {
          name: "已用",
          type: "bar",
          stack: "capacity",
          barMaxWidth: 30,
          data: used.map(value => ({ value, itemStyle: { color: usedColor(value), borderRadius: [5, 0, 0, 5] } })),
          label: { show: true, position: "inside", color: "#fff", fontWeight: 700, fontSize: 10, formatter: ({ value }: { value?: number | string }) => `${Number(value).toFixed(1)}%` }
        },
        {
          name: "可用",
          type: "bar",
          stack: "capacity",
          barMaxWidth: 30,
          data: available,
          itemStyle: { color: "#e9e3d8", borderRadius: [0, 5, 5, 0] }
        }
      ]
    };
    chart.current?.setOption(option, { notMerge: true });
  }, [disks]);

  return <div className="storage-chart" ref={host} style={{ height: `${Math.max(260, disks.length * 58 + 96)}px` }} aria-label="每个磁盘分区的空间占用图" />;
}

export default function StorageAnalysis({ disks }: { disks: DiskMetric[] }) {
  const total = disks.reduce((sum, disk) => sum + disk.totalBytes, 0);
  const available = disks.reduce((sum, disk) => sum + disk.availableBytes, 0);
  const used = Math.max(0, total - available);

  if (!disks.length) return <p className="empty">暂时没有读取到磁盘分区数据。</p>;

  return <>
    <div className="storage-overview">
      <article><span>分区数量</span><b>{disks.length}</b></article>
      <article><span>总容量</span><b>{formatBytes(total)}</b></article>
      <article><span>已经使用</span><b>{formatBytes(used)}</b></article>
      <article><span>剩余可用</span><b>{formatBytes(available)}</b></article>
    </div>
    <section className="storage-chart-card">
      <div><h4>磁盘空间占用</h4><small>颜色变红表示分区占用达到 90%</small></div>
      <StorageChart disks={disks} />
    </section>
    <div className="storage-disk-list">
      {disks.map(disk => {
        const percent = usagePercent(disk);
        const diskUsed = Math.max(0, disk.totalBytes - disk.availableBytes);
        return <article key={`${disk.name}-${disk.mountPoint}`} className={percent >= 90 ? "critical" : percent >= 75 ? "warning" : ""}>
          <div><b>{diskTitle(disk)}</b><span>{percent.toFixed(1)}% 已用</span></div>
          <i><em style={{ width: `${percent}%` }} /></i>
          <div className="storage-disk-values"><span>已用 {formatBytes(diskUsed)}</span><span>可用 {formatBytes(disk.availableBytes)}</span><span>总计 {formatBytes(disk.totalBytes)}</span></div>
          <small>实时读 {formatRate(disk.readBps)} · 写 {formatRate(disk.writeBps)}</small>
        </article>;
      })}
    </div>
  </>;
}
