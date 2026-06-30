//! Tool registry for the agent loop.
//!
//! Device tools are executed on the client (declared via Hello `capabilities`);
//! server-side tools (growth/history queries, later) would run on the server.
//! `available_tools` filters by the connected device's capabilities so the LLM
//! only ever sees tools that device can actually run.

use serde_json::json;

use crate::llm::ToolDef;

/// Is `name` a device-executed tool (vs a server-side one)?
pub fn is_device_tool(name: &str) -> bool {
    matches!(name, "notify" | "read_file")
}

/// Tools available this turn, filtered by the device's declared capabilities.
/// Server-side tools would be appended here regardless of capabilities.
pub fn available_tools(capabilities: &[String]) -> Vec<ToolDef> {
    let mut tools = Vec::new();
    for cap in capabilities {
        match cap.as_str() {
            "notify" => tools.push(ToolDef {
                name: "notify".into(),
                description: "在用户设备上弹出一条系统通知（提醒、打招呼等）。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "通知标题（可选）"},
                        "body": {"type": "string", "description": "通知正文"}
                    },
                    "required": ["body"]
                }),
            }),
            "read_file" => tools.push(ToolDef {
                name: "read_file".into(),
                description: "读取用户机器上一个文本文件的内容并返回文本。仅在用户明确给出文件路径时使用。"
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "文件的绝对路径"}
                    },
                    "required": ["path"]
                }),
            }),
            _ => {}
        }
    }
    tools
}
