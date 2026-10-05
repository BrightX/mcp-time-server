# Time MCP Server

基于 Rust 的提供时间和时区转换功能的模型上下文协议（MCP）服务器。该服务器使大语言模型（LLM）能够使用 IANA
时区名称获取当前时间信息并执行时区转换，并支持自动检测系统时区。

### 可用工具

- `get_current_time` - 获取特定时区或系统时区的当前时间。
    - **必需参数**：
        - `timezone` (字符串): IANA 时区名称（例如，'America/New_York'、'Europe/London'）
- `convert_time` - 在不同时区之间转换时间。
    - **必需参数**：
        - `source_timezone` (字符串): 源 IANA 时区名称
        - `time` (字符串): 24小时制格式的时间 (HH:MM)
        - `target_timezone` (字符串): 目标 IANA 时区名称

## License

[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE) 
