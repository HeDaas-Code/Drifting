# 兼容模型 API

在设置的「模型与 API」添加兼容连接，填写名称和服务商提供的 API 前缀，例如 https://relay.example/v1。选定 Chat Completions、Responses 或 Anthropic Messages。客户端在同一前缀上请求模型目录和对应的推理资源。可以使用目录里的模型，也可以在连接的设置里手动加入模型 ID。

连接资料只保存在当前设备。每个连接的密钥保存在系统安全存储，密钥绑定连接 ID、地址和协议。改地址或协议时需要为新地址重新填写密钥。远程地址使用 HTTPS。连接拒绝 HTTP 重定向。兼容 API 仍可使用写作工具，默认关闭托管搜索和厂商专用推理参数。

Rust 验收使用合成目录和流式回复，核对自定义前缀、模型 ID、凭据隔离和工具调用。运行 cargo test -p dr-agent-providers --locked。
