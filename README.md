# 大黄狗

一只住在 Windows 里的 AI 看门狗。当前仓库包含第一版桌面应用：真实系统资源采集、持续异常检测、进程观察、巡逻时间线，以及带安全确认的普通进程结束操作。

## 第一版能力

- 每 2 秒采集 Windows CPU、内存和进程资源；
- 采集磁盘读写和网络收发速率；
- 展示健康度、Top 进程和巡逻记录；
- 使用 SQLite 保存 7 天快照、巡逻事件和动作审计；
- CPU 或内存连续约 1 分钟高于 90% 后生成异常；
- 结束进程前校验 PID、创建时间和 30 秒确认令牌；
- Windows 关键进程命中拒绝清单后禁止结束；
- 操作后观察 15 秒，区分“执行成功”和“系统压力已改善”；
- 关闭主窗口后继续在系统托盘巡逻；
- 首次确认异常时发送 Windows 原生通知；
- 展示最近 4 分钟 CPU/内存趋势和最近 24 小时个人基线；
- 提供“电脑为什么卡”的纯本地规则诊断；
- 只读检查运行程序的 Authenticode 数字签名和路径风险；
- 扫描常见注册表与 Startup 文件夹中的开机启动项；
- 输出“看门报告”，坚持“未验证不等于恶意程序”；
- 支持打开进程文件位置和带确认的进程优先级调整；
- 采集父进程关系，将同一应用的主进程和子进程聚合展示；
- 可展开查看每个应用的成员进程、父 PID 和独立资源占用；
- 设置 CPU/内存阈值、采样频率、低功耗和通知开关；
- 设置历史保留周期，并可在明确确认后清除本地记忆；
- 浏览器开发模式使用演示数据，不允许执行系统操作。

## 开发环境

- Node.js 20+
- Rust stable
- Windows WebView2
- Visual Studio C++ Build Tools

```powershell
npm install
npm run tauri:dev
```

仅预览前端：

```powershell
npm run dev
```

运行检查：

```powershell
npm run build
cd src-tauri
cargo test
```

技术设计参见 [TECHNICAL_IMPLEMENTATION.md](./TECHNICAL_IMPLEMENTATION.md)。

## 安全说明

第一版不会执行任意 PowerShell/CMD，不会自动结束进程，也不会绕过用户确认。结束进程仍可能造成未保存数据丢失，请只对确认了解的普通应用执行。
