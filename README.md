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
