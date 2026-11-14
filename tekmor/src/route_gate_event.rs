#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Shell,
    Edit,
    RouteTask,
    Other,
}

pub fn classify_tool(tool: &str) -> ToolKind {
    let lower = tool.trim().to_ascii_lowercase();
    if is_route_task(&lower) {
        ToolKind::RouteTask
    } else if is_shell(&lower) {
        ToolKind::Shell
    } else if is_edit(&lower) {
        ToolKind::Edit
    } else {
        ToolKind::Other
    }
}

fn is_route_task(lower: &str) -> bool {
    lower == "route_task"
        || lower.ends_with(":route_task")
        || lower.ends_with("_route_task")
        || (lower.contains("route_task") && lower.contains("tekmor"))
}

fn is_shell(lower: &str) -> bool {
    matches!(
        lower,
        "bash"
            | "shell"
            | "zsh"
            | "sh"
            | "terminal"
            | "run_terminal_cmd"
            | "run_terminal"
            | "run_command"
            | "shell_command"
            | "execute"
            | "exec"
            | "powershell"
            | "pwsh"
            | "cmd"
    ) || lower.contains("shell")
        || lower.contains("terminal")
        || lower.ends_with("_bash")
        || lower.ends_with(".bash")
}

fn is_edit(lower: &str) -> bool {
    matches!(lower, "edit" | "searchreplace")
        || [
            "write",
            "strreplace",
            "search_replace",
            "apply_patch",
            "applypatch",
            "editnotebook",
            "edit_notebook",
            "fileedit",
            "replace_file_content",
            "write_to_file",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_tools_across_hosts_are_shell() {
        for tool in ["Bash", "run_terminal_cmd", "mcp__x__shell", "zsh", "pwsh"] {
            assert_eq!(classify_tool(tool), ToolKind::Shell, "{tool}");
        }
    }

    #[test]
    fn edit_tools_are_edit() {
        for tool in ["StrReplace", "write_to_file", "ApplyPatch", "edit"] {
            assert_eq!(classify_tool(tool), ToolKind::Edit, "{tool}");
        }
    }

    #[test]
    fn route_task_is_recognised_under_each_naming_scheme() {
        for tool in [
            "route_task",
            "mcp__tekmor__route_task",
            "mcp_tekmor_route_task",
            "tekmor:route_task",
        ] {
            assert_eq!(classify_tool(tool), ToolKind::RouteTask, "{tool}");
        }
    }

    #[test]
    fn other_tools_are_other() {
        for tool in ["Read", "grep_search", "file_summary", ""] {
            assert_eq!(classify_tool(tool), ToolKind::Other, "{tool}");
        }
    }
}
