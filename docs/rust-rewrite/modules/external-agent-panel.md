# 外部 Agent 侧栏

侧栏可以把当前会话绑定到内置 Agent、Codex App Server 或 DeepSeek Harness ACP。外部模式由本机的 `codex app-server --listen-stdio` 或 `dsh --profile acp` 提供会话、模型、工具调用、授权请求和语言行为；Drifting 只显示输入、输出、进度和授权界面。

外部进程通过 JSON-RPC over stdio 连接。会话建立顺序是 `initialize`、`session/new`、`session/prompt`；停止使用 `session/cancel`。外部 Agent 仍通过 Drifting 的 MCP 端点获得书籍工具，工具授权、书籍范围和写入回执继续由 Drifting 控制。

外部进程退出或协议错误时，侧栏显示重连操作，不会悄悄切换到内置模型。会话选择和外部命令配置属于设备设置，不把密钥、完整命令输出或本地路径写进书籍同步数据。
