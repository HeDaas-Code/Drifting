# 外部 Agent 侧栏

侧栏每个会话可选择内置、Codex App Server 或 DeepSeek Harness ACP。外部模式使用已安装的 `codex app-server --stdio` 或 `dsh --profile acp`，模型、登录、密钥、语言由各平台自己的配置管理；Drifting 不复制它们的凭据，也不在失败时改用内置模型。

两种协议明确分开：Codex 在 `initialize` / `initialized` 后调用 `thread/start` 或 `thread/resume`，以 `turn/start` 发消息、`turn/interrupt` 停止；ACP 以 `initialize`、`session/new` 或 `session/load`、`session/prompt`、`session/cancel` 交互。侧栏接收增量文本、工具进度、授权请求和结束事件，保存设备本地会话 ID；重新连接继续原作品的远端会话，另一部作品不会借用该会话。

外部平台通过现有 MCP 端点取得完整的公开写作工具和作品模板动作。每个侧栏会话持有只适用于起始作品的临时授权，仍走当前内容读取、写入屏障、冲突检查和 SQLite 写入回执。断线、撤权、切书或销毁面板会结束进程并撤销授权；已进入事务的写入依照共享 Agent Runtime 的结算边界处理，不重放未知结果。

多个授权请求按队列逐一显示。Codex 的 MCP 写入确认（`mcpServer/elicitation/request`）、命令审批和 ACP 权限选项均保留平台的答复格式；带表单字段的未知请求保留结构化答复入口。失败、中断和 ACP 取消不会显示成成功。Codex 启动一轮前收到停止，侧栏等到 turn ID 到达即发送中断。断开前保存已收到的分段文本，方便重新接入。

## 可重复验收

- `dr-platform/tests/external_agent.rs` 通过真实子进程和 NDJSON 管道，驱动合成的 Codex / ACP 对端，核对分段输出、授权应答、恢复、取消和断线。`Wire` 单元测试区分协议、终止原因及当前 ACP 请求 ID。
- `dr-view-agent` 的 GPUI 测试核对请求 ID 的双向命名空间、授权队列、部分输出保存、取消状态和不同书籍的会话隔离。
- `dr-mcp/tests/inbound.rs`、`tests/grants.rs` 和 `dr-view-agent/tests/external.rs` 验证真正的书目范围、撤权、断线结算和公开写作工具。
- 安装并配置外部平台后，显式运行 `cargo run --locked -p dr-mcp --example external-smoke -- codex` 或 `-- dsh`。此程序创建 Testkit Tiny 合成书库，启动当前 MCP 服务，用当前示例自身作 stdio 桥，仅允许在测试作品中创建一个条目，并核对写入回执、另一部作品未变及授权撤销。它不改外部平台的全局配置，不读作者书库；必须使用平台自己已配置的登录。

2026-11-20 本机 Codex CLI 0.162.1 的真实 MCP 写作验收通过。DSH 0.2.2-alpha.2 建立 ACP 会话成功，但当前 `deepseek-official` 路由缺少密钥，推理被平台拒绝；尚不能声明 DSH 真实写作通过，需作者在 DSH 中完成配置后重跑。
