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
    matches!(
        name,
        "notify"
            | "read_file"
            | "list_dir"
            | "search_files"
            | "get_clipboard"
            | "set_clipboard"
            | "system_info"
            | "write_file"
            | "open_url"
            | "open_path"
            | "run_command"
    )
}

fn obj(props: serde_json::Value, required: &[&str]) -> serde_json::Value {
    json!({ "type": "object", "properties": props, "required": required })
}

/// Tools available this turn, filtered by the device's declared capabilities.
pub fn available_tools(capabilities: &[String]) -> Vec<ToolDef> {
    let mut tools = Vec::new();
    let mut push = |name: &str, desc: &str, params: serde_json::Value| {
        tools.push(ToolDef { name: name.into(), description: desc.into(), parameters: params });
    };
    for cap in capabilities {
        match cap.as_str() {
            "notify" => push(
                "notify",
                "在用户设备上弹出一条系统通知（提醒、打招呼等）。",
                obj(
                    json!({
                        "title": {"type": "string", "description": "通知标题（可选）"},
                        "body": {"type": "string", "description": "通知正文"}
                    }),
                    &["body"],
                ),
            ),
            "read_file" => push(
                "read_file",
                "读取一个文本文件的内容并返回文本。仅在用户给出文件路径时使用。",
                obj(
                    json!({"path": {"type": "string", "description": "文件绝对路径（支持 ~）"}}),
                    &["path"],
                ),
            ),
            "list_dir" => push(
                "list_dir",
                "列出某个目录下的文件和子目录, 可选按关键词过滤文件名。",
                obj(
                    json!({
                        "path": {"type": "string", "description": "目录绝对路径（支持 ~, 如 ~/Downloads）"},
                        "filter": {"type": "string", "description": "可选: 只返回文件名包含此关键词的项(不区分大小写)"}
                    }),
                    &["path"],
                ),
            ),
            "search_files" => push(
                "search_files",
                "在某个目录下递归查找文件名包含关键词的文件(最多4层深, 返回前100个)。用于『全盘/某目录找 xx』。",
                obj(
                    json!({
                        "dir": {"type": "string", "description": "起始目录（支持 ~, 默认 ~）"},
                        "pattern": {"type": "string", "description": "文件名关键词(不区分大小写)"}
                    }),
                    &["pattern"],
                ),
            ),
            "get_clipboard" => push(
                "get_clipboard",
                "读取用户当前剪贴板的文本内容。",
                obj(json!({}), &[]),
            ),
            "set_clipboard" => push(
                "set_clipboard",
                "把一段文本写入用户的剪贴板。",
                obj(
                    json!({"text": {"type": "string", "description": "要写入剪贴板的文本"}}),
                    &["text"],
                ),
            ),
            "system_info" => push(
                "system_info",
                "获取设备基本信息(系统/用户/主目录/电量)。",
                obj(json!({}), &[]),
            ),
            "write_file" => push(
                "write_file",
                "把文本内容写入(覆盖)一个文件。用于记笔记/保存内容。请确认路径是用户期望的。",
                obj(
                    json!({
                        "path": {"type": "string", "description": "目标文件绝对路径（支持 ~）"},
                        "content": {"type": "string", "description": "要写入的文本内容"}
                    }),
                    &["path", "content"],
                ),
            ),
            "open_url" => push(
                "open_url",
                "用系统默认浏览器打开一个网址。",
                obj(
                    json!({"url": {"type": "string", "description": "要打开的 http(s) 网址"}}),
                    &["url"],
                ),
            ),
            "open_path" => push(
                "open_path",
                "用系统默认程序打开一个文件/文件夹/应用。",
                obj(
                    json!({"path": {"type": "string", "description": "要打开的路径（支持 ~）"}}),
                    &["path"],
                ),
            ),
            "run_command" => push(
                "run_command",
                "在用户机器上执行一条 shell 命令。⚠️ 高风险: 执行前会弹出系统对话框让用户确认, 用户拒绝则不执行。仅在确有必要且用户认可时使用。",
                obj(
                    json!({"command": {"type": "string", "description": "要执行的 shell 命令"}}),
                    &["command"],
                ),
            ),
            _ => {}
        }
    }
    tools
}
