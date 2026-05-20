use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use zellij_tile::prelude::*;

mod shim;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 插件状态。Zellij 不会向入口点传递持久实例，
/// 因此实际实例存放在 shim.rs 的 thread_local 中。
pub struct State {
    active_tab: Option<usize>,
    focused_pane: Option<PaneId>,
    im_select: String,
    state_dir: String,
    session_name: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            active_tab: None,
            focused_pane: None,
            im_select: String::new(),
            state_dir: String::new(),
            session_name: None,
        }
    }
}

/// 配置值中可能包含 "~"，而 sh 不会自动展开它。
fn expand_tilde(path: &str) -> String {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}

/// 对字符串做 shell 引号转义，使其可以安全地插入到 sh -c 脚本中。
/// 防止 im_select 或 state_dir 包含特殊字符时引发注入。
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace("'", "'\\''"))
}

/// 清理上一次 Zellij 会话遗留的 .ime 文件。
/// 否则对一个已不存在的 pane 恢复 IME 时，
/// 会使用几小时甚至几天前保存的值。
fn clear_old_session_state(dir: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("ime") {
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("clear_old_session_state: failed to remove {:?}: {}", path, e);
            }
        }
    }
}

/// 清理已不存在 session 的状态目录。
/// 当用户执行 delete-session 后，对应的 .ime 文件不会被自动删除，
/// 这里在插件加载时做一次清理。
fn clear_dead_session_states(state_dir: &str, live_sessions: &[SessionInfo]) {
    let live_names: std::collections::HashSet<&str> =
        live_sessions.iter().map(|s| s.name.as_str()).collect();

    let Ok(entries) = std::fs::read_dir(state_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !live_names.contains(name) {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                eprintln!(
                    "clear_dead_session_states: failed to remove {:?}: {}",
                    path, e
                );
            }
        }
    }
}

impl State {
    /// 向 state_dir/debug.log 追加一条带时间戳的日志。
    /// 用于在生产环境中排查 im-select 执行失败的问题。
    fn log(&self, msg: &str) {
        let log_path = format!("{}/debug.log", self.state_dir);
        let mut file = match OpenOptions::new().create(true).append(true).open(&log_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("log: failed to open {}: {}", log_path, e);
                return;
            }
        };

        let ts = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S");
        if let Err(e) = writeln!(file, "[{}] {}", ts, msg) {
            eprintln!("log: failed to write: {}", e);
        }
    }

    /// 获取当前 session 的状态文件目录。
    fn session_state_dir(&self) -> Option<String> {
        self.session_name
            .as_ref()
            .map(|n| format!("{}/{}", self.state_dir, n))
    }

    /// 解析并设置当前 session 名称，初始化 session 状态目录。
    fn resolve_session_name(&mut self) {
        // 优先从环境变量获取（最快，不需要额外权限）
        let env_vars = get_session_environment_variables();
        if let Some(name) = env_vars.get("ZELLIJ_SESSION_NAME") {
            if !name.is_empty() {
                self.set_session_name(name);
                return;
            }
        }

        // 备选：从 session list 获取
        if let Ok(list) = get_session_list() {
            if let Some(session) = list.live_sessions.iter().find(|s| s.is_current_session) {
                self.set_session_name(&session.name);
            }
        }
    }

    /// 设置 session 名称并初始化对应的目录。
    fn set_session_name(&mut self, name: &str) {
        if self.session_name.as_deref() == Some(name) {
            return;
        }
        self.session_name = Some(name.to_string());
        let dir = format!("{}/{}", self.state_dir, name);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!(
                "set_session_name: failed to create session dir {}: {}",
                dir, e
            );
        }
        clear_old_session_state(&dir);
        self.log(&format!("session name resolved: {}", name));
    }

    /// 首次 PaneUpdate 时 active_tab 可能还是 None，
    /// 因为 TabUpdate 事件可能还没到达。此时回退到扫描 manifest。
    fn resolve_tab_from_manifest(&mut self, manifest: &PaneManifest) {
        if self.active_tab.is_some() {
            return;
        }
        for (tab_pos, panes) in &manifest.panes {
            if panes.iter().any(|p| p.is_focused) {
                self.active_tab = Some(*tab_pos);
                break;
            }
        }
    }

    /// 查找焦点 pane 时优先选择浮动 pane，
    /// 因为浮动 pane 可以覆盖在普通 pane 上方并窃取焦点。
    fn focused_pane_id(&self, manifest: &PaneManifest) -> Option<PaneId> {
        let tab = self.active_tab?;
        let panes = manifest.panes.get(&tab)?;

        let p = panes
            .iter()
            .find(|p| p.is_focused && !p.is_plugin && p.is_floating)
            .or_else(|| panes.iter().find(|p| p.is_focused && !p.is_plugin))?;

        debug_assert!(!p.is_plugin);
        Some(PaneId::Terminal(p.id))
    }

    /// 先保存旧 pane 的当前输入法，再恢复新 pane 之前保存的输入法。
    /// 第一次聚焦某个 pane 时没有旧 pane，只做恢复。
    fn switch_ime(&self, old_id: Option<PaneId>, new_id: PaneId) {
        self.log(&format!("pane switch: old={:?}, new={:?}", old_id, new_id));

        let Some(dir) = self.session_state_dir() else {
            self.log("switch_ime: no session name");
            return;
        };

        let mut ctx = BTreeMap::new();
        ctx.insert("im_switch".to_string(), "1".to_string());

        let im = shell_quote(&self.im_select);
        let dir = shell_quote(&dir);

        match old_id {
            Some(old) => {
                let script = format!(
                    "set -e; \
                     mkdir -p {dir}; \
                     OLD=$({im}); \
                     printf '%s' \"$OLD\" > {dir}/\"$1\".ime; \
                     if [ -f {dir}/\"$2\".ime ]; then \
                         NEW=$(cat {dir}/\"$2\".ime); \
                         [ \"$OLD\" != \"$NEW\" ] && {im} \"$NEW\"; \
                     fi",
                );
                run_command(
                    &[
                        "sh",
                        "-c",
                        &script,
                        "--",
                        &file_name(&old),
                        &file_name(&new_id),
                    ],
                    ctx,
                );
            }
            None => {
                let script = format!(
                    "set -e; \
                     mkdir -p {dir}; \
                     if [ -f {dir}/\"$1\".ime ]; then \
                         {im} \"$(cat {dir}/\"$1\".ime)\"; \
                     fi",
                );
                run_command(&["sh", "-c", &script, "--", &file_name(&new_id)], ctx);
            }
        }
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, config: BTreeMap<String, String>) {
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/tmp"));

        self.im_select = expand_tilde(
            &config
                .get("im_select")
                .cloned()
                .unwrap_or_else(|| format!("{}/.local/bin/im-select", home)),
        );
        self.state_dir = expand_tilde(
            &config
                .get("state_dir")
                .cloned()
                .unwrap_or_else(|| format!("{}/.cache/zellij-ime", home)),
        );

        if let Err(e) = std::fs::create_dir_all(&self.state_dir) {
            eprintln!("load: failed to create state dir {}: {}", self.state_dir, e);
        }

        let log_path = format!("{}/debug.log", self.state_dir);
        if let Ok(meta) = std::fs::metadata(&log_path) {
            if meta.len() > 1_048_576 {
                let _ = std::fs::remove_file(&log_path);
            }
        }

        // 必须先请求权限再订阅事件，否则 Zellij 会拒绝投递这些事件。
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadSessionEnvironmentVariables,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::RunCommandResult,
            EventType::SessionUpdate,
        ]);

        // 尝试同步获取 session 名称。如果权限尚未批准，会在 PermissionRequestResult
        // 或 SessionUpdate 事件中再次尝试。
        self.resolve_session_name();
        self.log("plugin loaded");
    }

    fn render(&mut self, _rows: usize, _cols: usize) {}

    fn pipe(&mut self, _message: PipeMessage) -> bool {
        false
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                // 权限批准后，再次尝试获取 session 名称
                if self.session_name.is_none() {
                    self.resolve_session_name();
                }
            }
            Event::PermissionRequestResult(PermissionStatus::Denied) => {}
            Event::SessionUpdate(sessions, _) => {
                // 从 SessionUpdate 中提取当前 session 名称
                if self.session_name.is_none() {
                    if let Some(session) = sessions.iter().find(|s| s.is_current_session) {
                        self.set_session_name(&session.name);
                        // 清理已不存在 session 的状态目录
                        clear_dead_session_states(&self.state_dir, &sessions);
                    }
                }
            }
            Event::TabUpdate(tabs) => {
                if let Some(t) = tabs.iter().find(|t| t.active) {
                    self.active_tab = Some(t.position);
                }
            }
            Event::PaneUpdate(manifest) => {
                self.resolve_tab_from_manifest(&manifest);
                let new_id = match self.focused_pane_id(&manifest) {
                    Some(id) => id,
                    None => return false,
                };
                if self.focused_pane == Some(new_id) {
                    return false;
                }
                // session 名称尚未解析，延迟输入法切换
                if self.session_name.is_none() {
                    self.focused_pane = Some(new_id);
                    return false;
                }
                let old_id = self.focused_pane;
                self.focused_pane = Some(new_id);
                self.switch_ime(old_id, new_id);
            }
            // im_select 通过 run_command 异步执行，在这里检查退出码。
            Event::RunCommandResult(exit_code, _stdout, _stderr, context) => {
                if context.contains_key("im_switch") && exit_code != Some(0) {
                    self.log("im-select command failed");
                }
            }
            _ => {}
        }
        false
    }
}

/// 将 PaneId 编码为 "t{数字}" 或 "p{数字}"，用作 .ime 文件名。
fn file_name(id: &PaneId) -> String {
    match id {
        PaneId::Terminal(n) => format!("t{}", n),
        PaneId::Plugin(n) => format!("p{}", n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        std::env::set_var("HOME", "/home/user");
        assert_eq!(expand_tilde("~"), "/home/user");
        assert_eq!(expand_tilde("~/foo"), "/home/user/foo");
        assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
        assert_eq!(expand_tilde("relative"), "relative");
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("safe"), "'safe'");
        assert_eq!(shell_quote("it\'s"), "'it'\\''s'");
        assert_eq!(shell_quote("a'b'c"), "'a'\\''b'\\''c'");
    }

    #[test]
    fn test_file_name() {
        assert_eq!(file_name(&PaneId::Terminal(42)), "t42");
        assert_eq!(file_name(&PaneId::Plugin(7)), "p7");
    }

    #[test]
    fn test_chrono_format() {
        let dt = chrono::DateTime::from_timestamp(0, 0).unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "1970-01-01 00:00:00"
        );

        let dt = chrono::DateTime::from_timestamp(1_609_459_200, 0).unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2021-01-01 00:00:00"
        );
    }
}
