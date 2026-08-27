# 大黄狗技术实现文档

> 版本：0.1  
> 状态：初稿  
> 依据：[requirment.md](./requirment.md)  
> 目标平台：Windows 10 22H2、Windows 11  
> 推荐技术栈：React + TypeScript + Tauri 2 + Rust + Python + SQLite

## 1. 文档目标

本文将“大黄狗”从产品概念落实为可开发、可测试、可安全发布的 Windows 桌面应用。首要目标不是复刻任务管理器，而是完成以下最小闭环：

```text
系统采集 → 异常确认 → 原因解释 → 处置建议 → 用户授权 → 安全执行 → 效果验证
```

产品人格只负责表达与交互，事实判断、风险评级、权限校验和系统操作必须由确定性程序负责。任何 AI 输出都不能直接成为系统指令。

## 2. 需求分析

### 2.1 核心用户价值

用户不需要理解 CPU、Working Set、I/O Wait 等指标，也能知道：

1. 电脑是否真的异常，而不是一次短暂波动；
2. 哪些进程最可能导致问题；
3. 建议操作的收益和风险；
4. 操作完成后，问题是否得到改善。

### 2.2 核心功能域

| 产品概念 | 技术能力 | MVP |
| --- | --- | --- |
| 看家/巡逻 | CPU、内存、磁盘、网络、进程采集 | 是 |
| 闻味道 | 阈值、持续时间、个人基线异常检测 | 是 |
| 盯住目标 | 进程历史和资源归因 | 是 |
| 叫 | 应用内提醒、Windows 通知 | 是 |
| 听主人命令 | 自然语言诊断入口 | 是 |
| 咬住目标 | 结束进程等系统动作 | 仅低/中风险动作 |
| 看门 | 签名、启动项、陌生程序风险分析 | 第二阶段 |
| 找东西 | 文件和进程搜索 | 第二阶段 |
| 长期记忆 | 趋势、规律、个性化基线 | MVP 简化版 |
| 自动优化 | 规则和定时动作 | 暂不进入 MVP |

### 2.3 MVP 范围

MVP 提供：

- 系统托盘与主窗口；
- CPU、内存、磁盘、网络以及进程级 CPU/内存监控；
- 最近 24 小时巡逻时间线；
- 持续高 CPU、持续高内存、单进程异常增长检测；
- “电脑为什么卡”的本地诊断；
- 查看进程详情、打开文件位置、结束普通用户进程；
- 操作前风险分级与明确确认；
- 操作后 30～60 秒效果验证；
- SQLite 本地历史数据和基础个人基线；
- 无 LLM 时可完整运行的规则诊断。

MVP 不提供：

- 杀毒软件式恶意程序定性；
- 驱动层监控、内核拦截和网络包捕获；
- 无确认的自动结束进程、删除文件或修改注册表；
- 跨设备同步；
- 依赖云端 LLM 才能工作的核心诊断；
- 自动处置系统服务、关键进程和其他用户的进程。

### 2.4 非功能要求

| 项目 | 目标 |
| --- | --- |
| 后台 CPU | 空闲时平均小于 1% |
| 后台内存 | MVP 目标小于 150 MB，不含本地模型 |
| 采样间隔 | 前台 2 秒，后台 5 秒，低功耗 15 秒 |
| UI 首屏 | 冷启动 3 秒内可交互 |
| 数据保留 | 原始快照 7 天，小时聚合 90 天，事件长期保留 |
| 离线能力 | 监控、检测、规则诊断、执行和验证均可离线 |
| 隐私 | 默认不上传进程路径、用户名、命令行和遥测数据 |
| 可恢复性 | 分析服务或 UI 崩溃不得中断采集数据库的一致性 |

## 3. 总体架构

```text
┌────────────────────────────────────────────────────────────┐
│ React + TypeScript                                         │
│ 狗窝首页 / 巡逻记录 / 异常详情 / 对话 / 授权确认            │
└──────────────────────────┬─────────────────────────────────┘
                           │ Tauri command / event
┌──────────────────────────▼─────────────────────────────────┐
│ Tauri + Rust Core                                          │
│ App Shell │ Collector │ Rule Engine │ Action Guard         │
│ State Machine │ Repository │ Notification │ Audit Log       │
└───────────────┬───────────────────────┬─────────────────────┘
                │                       │ localhost IPC
          Windows API                   ▼
                │              ┌──────────────────────┐
                │              │ Python Brain Sidecar │
                │              │ 特征/基线/高级分析   │
                │              └──────────┬───────────┘
                │                         │ 可选 LLM Provider
                ▼                         ▼
       Windows / ETW / PDH          Local or Cloud LLM
                │
                ▼
             SQLite
```

### 3.1 架构原则

1. **Rust 是可信执行边界**：采集、权限检查、动作执行和审计全部在 Rust 内完成。
2. **规则先于 AI**：告警事实和风险等级由规则或模型输出结构化结论，LLM 只解释，不改写事实。
3. **Python 可失效**：侧车不可用时降级到 Rust 规则引擎，不影响基本监控。
4. **本地优先**：遥测和历史默认只落本地 SQLite。
5. **最小权限**：应用默认以普通用户运行；确需管理员权限的单次动作通过独立提权执行器完成。
6. **命令白名单**：禁止 LLM 生成并直接执行 PowerShell、CMD 或任意脚本。

### 3.2 进程模型

| 进程 | 权限 | 职责 |
| --- | --- | --- |
| `dahuangdog.exe` | 普通用户 | Tauri UI、托盘、编排、普通采集与动作 |
| `dahuang-brain.exe` | 普通用户，低完整性优先 | Python 打包侧车，高级分析，可重启 |
| `dahuang-elevated.exe` | 按需管理员 | 只执行签名动作清单中的单次提权操作 |

MVP 可先不实现常驻 Windows Service。只有在“用户未登录时监控”或“跨会话管理”成为明确需求后再引入服务，避免过早增加安装、升级和权限复杂度。

## 4. 技术选型

### 4.1 前端

- React 18+、TypeScript；
- Vite 构建；
- Zustand 或 Redux Toolkit 管理 UI 状态，MVP 推荐 Zustand；
- TanStack Query 管理命令查询和缓存；
- 图表使用轻量 Canvas/SVG 库，限制可视点数量；
- 动画必须支持 Windows“减少动画”设置。

### 4.2 桌面与系统层

- Tauri 2：窗口、托盘、通知、自动更新和 IPC；
- Rust stable，异步运行时 Tokio；
- `windows` crate 调用 Win32/COM API；
- `sysinfo` 可用于原型，但关键指标应逐步替换为 Windows 原生采集；
- `rusqlite` + bundled SQLite，或 SQLx SQLite；MVP 推荐 `rusqlite`，减少异步数据库复杂度；
- `serde` 统一跨层数据契约；
- `tracing` 提供结构化本地日志。

### 4.3 分析层

- Python 3.12；
- NumPy、pandas 仅在确有需要时引入，优先使用轻依赖实现；
- scikit-learn 用于 Isolation Forest 等可选模型；
- PyInstaller/Nuitka 打包为侧车；
- Rust 与 Python 通过 localhost Named Pipe 通信，开发期可使用 stdio JSON Lines；
- 消息使用带版本号的 JSON，后期吞吐不足时再迁移 MessagePack/Protobuf。

Python 不应参与每 2 秒一次的基础采集。Rust 负责采集和预聚合，仅在事件触发或低频批处理时把特征发送给 Python，以降低资源占用和部署风险。

## 5. 仓库结构建议

```text
DahuangDog/
├─ apps/
│  └─ desktop/                 # React/Tauri 应用
│     ├─ src/
│     │  ├─ features/home/
│     │  ├─ features/patrol/
│     │  ├─ features/diagnosis/
│     │  ├─ features/actions/
│     │  ├─ components/
│     │  └─ contracts/
│     └─ src-tauri/
│        ├─ src/collector/
│        ├─ src/detector/
│        ├─ src/actions/
│        ├─ src/state/
│        ├─ src/storage/
│        ├─ src/brain/
│        └─ migrations/
├─ services/
│  └─ brain/
│     ├─ dahuang_brain/
│     │  ├─ features/
│     │  ├─ baseline/
│     │  ├─ diagnosis/
│     │  └─ llm/
│     └─ tests/
├─ packages/
│  └─ contracts/               # JSON Schema 与生成类型
├─ docs/
└─ tests/
   ├─ fixtures/
   └─ e2e/
```

## 6. 核心模块设计

### 6.1 Collector：系统采集

采集器输出统一 `SystemSnapshot`，避免 UI 或分析层直接调用系统 API。

```ts
interface SystemSnapshot {
  schemaVersion: 1;
  capturedAt: string;
  bootId: string;
  cpu: { totalPercent: number; perCorePercent?: number[] };
  memory: { usedBytes: number; totalBytes: number; pressure: number };
  disk: { readBps: number; writeBps: number; busyPercent?: number };
  network: { receiveBps: number; sendBps: number };
  gpu?: { utilizationPercent: number; memoryUsedBytes?: number };
  processes: ProcessSample[];
}
```

数据源建议：

| 指标 | 首选数据源 | 备注 |
| --- | --- | --- |
| CPU/内存 | PDH、`GetSystemTimes`、`GlobalMemoryStatusEx` | 低成本轮询 |
| 进程 | Toolhelp/`NtQuerySystemInformation`/PDH | 记录 PID + 创建时间，避免 PID 复用 |
| 磁盘/网络 | PDH 或 ETW 聚合 | MVP 先系统级 |
| GPU | PDH GPU Engine counters | 不支持时隐藏指标 |
| 签名 | WinVerifyTrust | 事件触发，不做高频扫描 |
| 启动项 | 注册表与 Startup 文件夹 | 第二阶段 |

采集调度：

- UI 打开：每 2 秒采集；
- 托盘后台：每 5 秒采集；
- 低功耗/电池模式：每 15 秒采集；
- 原始进程详情仅保留 Top N 与异常进程，避免数据库爆炸；
- 写入前按 10 秒窗口聚合，内存保留短时环形缓冲区。

### 6.2 Detector：异常检测

检测分三层，按顺序执行：

1. **硬规则**：高 CPU、高内存、磁盘繁忙等持续阈值；
2. **个人基线**：同一小时/工作负载下的 EWMA、Median/MAD 偏离；
3. **高级模型**：Python 的多变量异常检测，仅提供补充置信度。

MVP 初始规则：

| 规则 | 触发条件 | 恢复条件 |
| --- | --- | --- |
| `cpu.sustained_high` | CPU ≥ 90%，持续 5 分钟 | CPU < 70%，持续 1 分钟 |
| `memory.pressure` | 内存 ≥ 90%，持续 2 分钟 | 内存 < 80%，持续 1 分钟 |
| `process.cpu_high` | 单进程 CPU ≥ 50%，持续 3 分钟 | 低于 25%，持续 1 分钟 |
| `process.memory_growth` | 30 分钟增长 ≥ 1 GB 且未回落 | 增长停止或进程退出 |
| `disk.sustained_busy` | 磁盘繁忙 ≥ 90%，持续 3 分钟 | 低于 60%，持续 1 分钟 |

所有规则需有迟滞、冷却时间和事件合并，防止通知风暴。阈值后续应基于硬件能力和个人基线调整。

```rust
pub struct Finding {
    pub id: String,
    pub kind: FindingKind,
    pub severity: Severity,
    pub confidence: f32,
    pub first_seen_at: DateTime<Utc>,
    pub evidence: Vec<Evidence>,
    pub suspected_processes: Vec<ProcessIdentity>,
}
```

### 6.3 Diagnosis：根因分析

诊断不等同于让 LLM 阅读全部遥测。Rust 先生成结构化诊断上下文：

```text
异常类型 + 时间窗口 + 当前指标 + 基线差异
+ Top 进程贡献 + 最近进程变化 + 相关历史事件
```

确定性归因算法：

1. 计算异常窗口内各进程资源贡献；
2. 对比异常前基线，计算增量贡献；
3. 排除已知系统空闲、采集器自身等噪声；
4. 生成最多 3 个候选原因和证据；
5. 规则引擎给出处置选项；
6. 可选 LLM 将结构化结论改写为“大黄狗”语言。

LLM 必须返回受约束结构：

```json
{
  "summary": "主人，我发现电脑有点累。",
  "factsUsed": ["finding-123", "evidence-456"],
  "suggestedActionIds": ["inspect_process", "terminate_process"],
  "uncertainty": "medium"
}
```

后端校验 `factsUsed` 和 `suggestedActionIds`；未知 ID、超出白名单或与风险策略冲突的内容一律丢弃。

### 6.4 Action Guard：动作安全网关

UI、Python 和 LLM 都不能直接操作 Windows，只能向 Action Guard 提交类型化请求。

```rust
pub enum ActionRequest {
    OpenFileLocation { process: ProcessIdentity },
    SetProcessPriority { process: ProcessIdentity, level: Priority },
    TerminateProcess { process: ProcessIdentity },
}
```

执行流程：

```text
请求 → 参数解析 → 重新读取目标 → 风险评级 → 策略判断
→ 展示影响与证据 → 用户确认 → 执行 → 审计 → 验证
```

风险等级：

| 等级 | 示例 | 策略 |
| --- | --- | --- |
| R0 只读 | 查看详情、打开位置 | 可直接执行 |
| R1 可逆 | 调低普通进程优先级 | 首次确认，可记住偏好 |
| R2 有损 | 结束普通进程，可能丢失数据 | 每次明确确认 |
| R3 高危 | 停服务、改启动项、结束提权进程 | 二次确认并按需提权 |
| R4 禁止 | 关键系统进程、任意命令、删除系统文件 | 产品内拒绝 |

关键保护：

- 使用 PID + 进程创建时间 + 可执行文件规范路径校验身份；
- 维护 Windows 关键进程和受保护进程拒绝清单；
- 执行前再次检查签名、会话所有者和完整性级别；
- 确认令牌绑定动作类型、目标、参数和短有效期，防止 TOCTOU；
- 任何动作写入不可由 UI 修改的审计日志；
- 不提供“永久允许 AI 自动结束进程”选项。

### 6.5 Verifier：效果验证

动作成功仅代表系统调用返回成功，不代表问题解决。每个动作定义验证器：

| 动作 | 技术成功 | 效果成功 |
| --- | --- | --- |
| 结束进程 | 目标 PID 已退出 | CPU/内存压力在 60 秒内明显下降 |
| 调低优先级 | 优先级读取值已变化 | 前台响应或 CPU 争用改善 |
| 禁用启动项 | 配置项已变更 | 下次登录未自动启动 |

结果分为 `succeeded`、`partially_improved`、`no_improvement`、`failed`，并保留验证前后证据。

### 6.6 Dog State Machine

```text
Sleep ↔ Idle → Patrol → Suspicious → Investigating
                       ↘ Normal       ↙       ↘
                                  Safe       Risk
                                               ↓
                                        AwaitingApproval
                                               ↓
                                           Acting
                                               ↓
                                          Verifying
                                               ↓
                                      Resolved / Unresolved
```

状态机由 Rust 持有唯一真相，UI 只订阅状态事件。关键转换：

| 当前状态 | 事件 | 下一状态 |
| --- | --- | --- |
| Patrol | 规则连续触发 | Suspicious |
| Suspicious | 达到确认窗口 | Investigating |
| Investigating | 无风险 | Safe |
| Investigating | 形成处置建议 | AwaitingApproval |
| AwaitingApproval | 用户批准且令牌有效 | Acting |
| Acting | 系统调用完成 | Verifying |
| Verifying | 指标恢复 | Resolved |
| Verifying | 超时未恢复 | Unresolved |

每次转换都生成 `domain_event`，驱动时间线、动画、通知和审计，避免多个模块各自推断状态。

## 7. 数据与存储

### 7.1 SQLite 表

```sql
CREATE TABLE system_snapshots (
  id INTEGER PRIMARY KEY,
  captured_at INTEGER NOT NULL,
  boot_id TEXT NOT NULL,
  cpu_percent REAL NOT NULL,
  memory_used_bytes INTEGER NOT NULL,
  memory_total_bytes INTEGER NOT NULL,
  disk_read_bps INTEGER NOT NULL,
  disk_write_bps INTEGER NOT NULL,
  net_recv_bps INTEGER NOT NULL,
  net_send_bps INTEGER NOT NULL
);

CREATE TABLE process_samples (
  id INTEGER PRIMARY KEY,
  snapshot_id INTEGER NOT NULL,
  pid INTEGER NOT NULL,
  process_started_at INTEGER NOT NULL,
  name TEXT NOT NULL,
  executable_path_hash TEXT,
  cpu_percent REAL NOT NULL,
  working_set_bytes INTEGER NOT NULL,
  FOREIGN KEY(snapshot_id) REFERENCES system_snapshots(id) ON DELETE CASCADE
);

CREATE TABLE findings (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  severity TEXT NOT NULL,
  confidence REAL NOT NULL,
  status TEXT NOT NULL,
  first_seen_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  evidence_json TEXT NOT NULL
);

CREATE TABLE domain_events (
  id TEXT PRIMARY KEY,
  occurred_at INTEGER NOT NULL,
  event_type TEXT NOT NULL,
  finding_id TEXT,
  public_payload_json TEXT NOT NULL
);

CREATE TABLE action_audits (
  id TEXT PRIMARY KEY,
  requested_at INTEGER NOT NULL,
  action_type TEXT NOT NULL,
  target_json TEXT NOT NULL,
  risk_level TEXT NOT NULL,
  approval_json TEXT,
  result_json TEXT,
  verification_json TEXT
);

CREATE TABLE baselines (
  metric_key TEXT NOT NULL,
  time_bucket INTEGER NOT NULL,
  sample_count INTEGER NOT NULL,
  median REAL NOT NULL,
  mad REAL NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(metric_key, time_bucket)
);
```

路径和命令行可能包含用户名或敏感参数。默认数据库只保存规范化哈希；只有用户进入详情且明确需要时才临时读取原文。数据库启用 WAL、外键和迁移版本。

### 7.2 数据生命周期

- 10 秒聚合快照：7 天；
- 5 分钟聚合：30 天；
- 1 小时聚合：90 天；
- 普通进程样本：随快照清理；
- Finding、用户动作和验证结果：默认 180 天，可手动清空；
- 应用设置提供“一键清除所有本地记忆”。

## 8. 跨层接口

### 8.1 Tauri Commands

```text
get_current_status() -> CurrentStatus
get_metric_series(metric, range) -> MetricSeries
list_findings(filter) -> FindingSummary[]
get_finding(id) -> FindingDetail
request_diagnosis(question?) -> Diagnosis
prepare_action(request) -> ActionPreview
confirm_action(preview_id, confirmation_token) -> ActionExecution
get_action_result(id) -> ActionResult
update_settings(patch) -> Settings
```

`confirm_action` 不能接受 UI 自行拼装的目标参数，只能引用服务端已生成且未过期的 `preview_id`。

### 8.2 Tauri Events

```text
telemetry://summary
dog://state-changed
finding://created
finding://updated
action://progress
notification://requested
brain://availability-changed
```

所有消息包含 `schemaVersion`、`eventId`、`occurredAt`。前端收到未知版本时忽略并记录日志，不尝试猜测字段。

### 8.3 Rust/Python 协议

Python 仅暴露：

```text
health
analyze_features
update_baseline
explain_diagnosis
```

请求包含最少必要数据，禁止传递用户文件内容。每次调用设置 5～15 秒超时、最大消息体和并发限制。失败时熔断并回退到 Rust 规则结果。

## 9. UI 信息架构

### 9.1 狗窝首页

- 大黄狗当前状态与一句话结论；
- 健康度和 CPU、内存、磁盘、网络四项摘要；
- 当前最重要的一个发现；
- “让我看看”进入诊断；
- 今日巡逻摘要。

健康度只能作为解释性汇总，不应掩盖具体风险。建议由确定性公式计算，并展示扣分原因。

### 9.2 巡逻记录

时间线只显示有意义的领域事件，不展示每次采样。支持按“异常、调查、动作、结果”筛选。

### 9.3 异常详情

固定信息层级：

1. 人格化结论；
2. 持续时间、指标和趋势等证据；
3. 可能原因及置信度；
4. 推荐动作、预期收益和风险；
5. 查看详情或确认操作。

### 9.4 对话入口

首版只支持受限意图：

- 为什么电脑卡；
- 谁占用 CPU/内存最多；
- 最近发生了什么；
- 查看某个已识别异常；
- 请求一个白名单动作。

无法映射到已知意图时返回说明，不执行自由文本命令。

## 10. AI 与提示词安全

LLM 输入由以下部分组成：

- 固定角色和语言规范；
- 结构化诊断事实；
- 允许推荐的动作 ID；
- 不确定性要求；
- 输出 JSON Schema。

不得将以下内容作为可信指令：进程名、窗口标题、文件内容、命令行参数、网页文本。它们均属于不可信数据，必须转义、截断并与系统提示隔离，以防提示注入。

LLM 不负责：

- 判定程序一定是病毒；
- 生成可执行 shell 命令；
- 决定是否跳过确认；
- 修改风险等级；
- 直接调用 Windows API；
- 虚构未在证据中的指标或原因。

云端模型为可选能力。启用前需明确告知将上传哪些经过脱敏的数据，并提供本地模型或纯规则模式。

## 11. Windows 权限与安全

### 11.1 权限策略

- 默认普通用户启动，不申请管理员权限；
- 只采集当前会话可访问的信息；
- 需要提权时由独立执行器弹出 UAC；
- 提权执行器校验父进程、请求签名、随机 nonce、过期时间和动作白名单；
- 不保存管理员令牌，不建立通用高权限 RPC 通道。

### 11.2 安装与更新

- 安装包和可执行文件必须代码签名；
- 自动更新包必须验签，支持失败回滚；
- Python 侧车和模型文件纳入完整性清单；
- Content Security Policy 禁止远程脚本；
- Tauri 命令按最小能力暴露，不启用通用 shell 插件。

### 11.3 日志与隐私

- 日志默认不记录完整路径、命令行、用户问题原文和访问令牌；
- 诊断包导出前展示内容并二次确认；
- API Key 使用 Windows Credential Manager/DPAPI，不写入 SQLite 或日志；
- 崩溃报告默认本地生成，上传需用户同意。

## 12. 可观测性与容错

- 每个采集周期记录耗时、丢弃样本数和数据库写入延迟；
- Python 侧车有心跳、指数退避重启和熔断；
- 数据库写入使用单写者队列，队列满时优先丢弃普通快照，不丢事件与动作审计；
- UI 与后端断开时显示“暂时没听见”，不得显示旧数据为实时状态；
- 应用非正常退出后，下次启动恢复未完成动作的验证，但不自动重做动作。

## 13. 测试策略

### 13.1 单元测试

- 采样差值、进程 CPU 计算和 PID 复用处理；
- 阈值持续窗口、迟滞、冷却和事件合并；
- 风险评级与关键进程拒绝策略；
- 状态机所有合法/非法转换；
- 数据保留与聚合；
- LLM 输出 Schema 和事实引用校验。

### 13.2 集成测试

- 使用固定遥测序列回放高 CPU、内存增长和磁盘繁忙场景；
- Rust/Python 超时、崩溃、错误消息和版本不匹配；
- SQLite 升级迁移、断电恢复和并发读取；
- 操作预览过期、目标进程退出、PID 被复用；
- 普通权限与 UAC 拒绝路径。

### 13.3 端到端测试

```text
启动测试负载
→ 规则持续触发
→ UI 出现异常
→ 展示正确证据
→ 用户批准动作
→ 目标被处理
→ 指标恢复
→ 时间线显示已解决
```

任何会结束进程的自动化测试只能操作测试程序，禁止按名称模糊匹配真实系统进程。

### 13.4 验收标准

MVP 可验收的关键场景：

1. 连续高 CPU 被识别，短时尖峰不告警；
2. 用户能看到前三个贡献进程及数据依据；
3. 不连接 LLM 也能生成可理解的诊断；
4. 结束普通测试进程前必须确认；
5. 对关键系统进程的结束请求被拒绝；
6. 操作后能区分“执行成功”和“问题已改善”；
7. 重启应用后巡逻记录和审计仍可查询；
8. 后台资源占用达到非功能目标。

## 14. 开发阶段

### 阶段 0：工程骨架（1 周）

- 初始化 React、Tauri、Rust workspace；
- 建立共享契约、迁移、日志和 CI；
- 完成托盘、单实例和基础窗口。

### 阶段 1：能看（2～3 周）

- 系统/进程采集；
- SQLite 时序存储与清理；
- 狗窝首页实时摘要；
- 基准性能与长稳测试。

### 阶段 2：能闻（2 周）

- 规则引擎、持续窗口、迟滞；
- Finding 生命周期；
- Dog State Machine；
- 巡逻记录和 Windows 通知。

### 阶段 3：能解释（2 周）

- 进程贡献和根因排序；
- 本地模板化语言；
- Python 侧车与个人基线；
- 可选 LLM 解释层。

### 阶段 4：能处理（2 周）

- Action Guard、操作预览、确认令牌；
- 普通进程动作和拒绝清单；
- 验证器和动作审计；
- UAC 流程原型。

### 阶段 5：发布准备（1～2 周）

- 安装、签名、更新、回滚；
- 隐私设置与诊断包；
- E2E、安全审查、资源占用优化；
- Windows 10/11 兼容矩阵测试。

## 15. 后续演进

MVP 稳定后按价值与风险推进：

1. 数字签名、启动项和陌生程序的“看门报告”；
2. GPU 进程归因、服务与计划任务诊断；
3. 更准确的个人基线和周期规律发现；
4. 文件/进程搜索；
5. 本地知识库与 Windows 官方文档 RAG；
6. 用户可配置但仍需安全确认的优化方案；
7. 常驻服务，仅用于确有需求的未登录监控。

## 16. 关键决策记录

| 决策 | 选择 | 原因 |
| --- | --- | --- |
| 基础检测是否依赖 AI | 不依赖 | 保证离线、稳定、可解释 |
| Python 是否承担高频采集 | 否 | 降低资源占用和部署复杂度 |
| LLM 是否能直接执行动作 | 不能 | 防止幻觉、注入和越权 |
| 首版是否安装常驻服务 | 否 | 减少权限面和安装复杂度 |
| 是否记录完整进程路径 | 默认不记录 | 降低隐私风险 |
| 是否自动结束异常进程 | 否 | 符合“聪明但不自作聪明” |
| UI 状态由谁决定 | Rust 状态机 | 保持跨模块一致性 |

## 17. 首个开发切片

建议第一个可演示切片只贯通一条路径：

```text
Rust 每 2 秒采集 CPU/内存和 Top 进程
→ React 狗窝展示实时状态
→ 连续 60 秒测试高 CPU 触发 Finding
→ 时间线显示“发现—调查—原因”
→ 用户确认结束测试进程
→ 30 秒后比较前后 CPU
→ 显示“问题已改善/没有明显改善”
```

该切片同时验证采集、存储、检测、状态机、IPC、授权、执行、验证和产品语言，是进入完整功能开发前最有价值的架构验证。

