use std::collections::BTreeMap;
use zellij_tile::prelude::*;

mod shim;
mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 插件状态。Zellij 不会向入口点传递持久实例，
/// 因此实际实例存放在 shim.rs 的 thread_local 中。
#[derive(Default)]
pub struct State {
    active_tab: Option<usize>,
    focused_pane: Option<PaneId>,
    im_select: String,
    state_dir: String,
    session_name: Option<String>,
    /// 上一次 SessionUpdate 中的 connected_clients，用于检测 attach/detach
    last_connected_clients: Option<usize>,
}

impl State {
    /// 获取当前 session 的状态文件目录。
    fn session_state_dir(&self) -> Option<String> {
        self.session_name
            .as_ref()
            .map(|n| format!("{}/{}", self.state_dir, n))
    }

    /// 从 ModeUpdate 中提取 session 名称。
    fn handle_mode_update(&mut self, mode_info: &ModeInfo) {
        if self.session_name.is_some() {
            return;
        }
        if let Some(name) = &mode_info.session_name {
            if !name.is_empty() {
                self.set_session_name(name);
            }
        }
    }

    /// 设置 session 名称。
    /// 如果此前已经收到了 PaneUpdate 并记录了 focused_pane，
    /// 立即恢复该 pane 的输入法（因为 PaneUpdate 阶段 session_name 尚未解析，
    /// 所以当时跳过了 switch_ime）。
    fn set_session_name(&mut self, name: &str) {
        if self.session_name.as_deref() == Some(name) {
            return;
        }
        self.session_name = Some(name.to_string());
        eprintln!("session name resolved: {}", name);
        if let Some(pane) = self.focused_pane {
            self.switch_ime(None, pane);
        }
    }

    /// 清理已不存在 session 的状态目录。
    /// 当用户执行 delete-session 后，对应的 .ime 文件不会被自动删除，
    /// 这里在收到 SessionUpdate 时做一次清理。
    fn clear_dead_session_states(&self, live_sessions: &[SessionInfo]) {
        // 空列表保护：SessionUpdate 在启动初期或权限被拒时可能送达空列表，
        // 此时若执行清理脚本，下面的 for 循环不进入，所有目录都会被 rm -rf。
        if live_sessions.is_empty() {
            return;
        }
        let state_dir = util::shell_quote(&self.state_dir);
        // 白名单通过位置参数传入，避免 shell 解析风险。
        let script = format!(
            "state_dir={state_dir}; \
             [ -d \"$state_dir\" ] || exit 0; \
             for d in \"$state_dir\"/*; do \
                 [ -d \"$d\" ] || continue; \
                 name=$(basename \"$d\"); \
                 keep=0; \
                 for live in \"$@\"; do \
                     [ \"$name\" = \"$live\" ] && keep=1 && break; \
                 done; \
                 [ \"$keep\" = 0 ] && rm -rf \"$d\"; \
             done"
        );
        let mut args: Vec<&str> = vec!["sh", "-c", &script, "--"];
        for s in live_sessions {
            args.push(&s.name);
        }
        let mut ctx = BTreeMap::new();
        ctx.insert("im_cleanup".to_string(), "1".to_string());
        run_command(&args, ctx);
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

    /// 保存当前焦点 pane 的输入法到 .ime 文件。
    /// 用于 BeforeClose 等需要保存当前状态但不切换 pane 的场景。
    fn save_current_ime(&self) {
        let Some(dir) = self.session_state_dir() else {
            return;
        };
        let Some(pane_id) = self.focused_pane else {
            return;
        };

        let mut ctx = BTreeMap::new();
        ctx.insert("im_save".to_string(), "1".to_string());

        let im = util::shell_quote(&self.im_select);
        let dir = util::shell_quote(&dir);
        let script = format!(
            "set -e; \
             mkdir -p {dir}; \
             OLD=$({im}); \
             printf '%s' \"$OLD\" > {dir}/\"$1\".ime",
        );
        run_command(
            &["sh", "-c", &script, "--", &util::file_name(&pane_id)],
            ctx,
        );
    }

    /// 先保存旧 pane 的当前输入法，再恢复新 pane 之前保存的输入法。
    /// 第一次聚焦某个 pane 时没有旧 pane，只做恢复。
    fn switch_ime(&mut self, old_id: Option<PaneId>, new_id: PaneId) {
        eprintln!("pane switch: old={:?}, new={:?}", old_id, new_id);

        let Some(dir) = self.session_state_dir() else {
            eprintln!("switch_ime: no session name");
            return;
        };

        let mut ctx = BTreeMap::new();
        ctx.insert("im_switch".to_string(), "1".to_string());

        let im = util::shell_quote(&self.im_select);
        let dir = util::shell_quote(&dir);

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
                        &util::file_name(&old),
                        &util::file_name(&new_id),
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
                run_command(&["sh", "-c", &script, "--", &util::file_name(&new_id)], ctx);
            }
        }
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, config: BTreeMap<String, String>) {
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/tmp"));

        self.im_select = util::expand_tilde(
            &config
                .get("im_select")
                .cloned()
                .unwrap_or_else(|| "im-select".to_string()),
        );
        self.state_dir = util::expand_tilde(
            &config
                .get("state_dir")
                .cloned()
                .unwrap_or_else(|| format!("{}/.cache/zellij-ime", home)),
        );

        // 必须先请求权限再订阅事件，否则 Zellij 会拒绝投递这些事件。
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::RunCommandResult,
            EventType::ModeUpdate,
            EventType::SessionUpdate,
            EventType::BeforeClose,
        ]);
        eprintln!("plugin loaded");
    }

    fn render(&mut self, _rows: usize, _cols: usize) {}

    fn pipe(&mut self, _message: PipeMessage) -> bool {
        false
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::ModeUpdate(mode_info) => {
                self.handle_mode_update(&mode_info);
            }
            Event::SessionUpdate(sessions, _) => {
                if let Some(session) = sessions.iter().find(|s| s.is_current_session) {
                    let prev = self.last_connected_clients;
                    let curr = session.connected_clients;
                    self.last_connected_clients = Some(curr);

                    // 首次获取 session_name
                    if self.session_name.is_none() {
                        self.set_session_name(&session.name);
                    }

                    // 客户端从有到无：detach 了，保存当前输入法
                    if prev.map_or(false, |n| n > 0) && curr == 0 {
                        self.save_current_ime();
                    }

                    // 客户端从无到有：attach 了，恢复当前 pane 的输入法
                    if prev == Some(0) && curr > 0 {
                        if let Some(pane) = self.focused_pane {
                            if self.session_name.is_some() {
                                self.switch_ime(None, pane);
                            }
                        }
                    }
                }
                self.clear_dead_session_states(&sessions);
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
            Event::BeforeClose => {
                self.save_current_ime();
            }
            // im_select 通过 run_command 异步执行，在这里检查退出码。
            Event::RunCommandResult(exit_code, _stdout, _stderr, context) => {
                if (context.contains_key("im_switch") || context.contains_key("im_save"))
                    && exit_code != Some(0)
                {
                    eprintln!("im-select command failed");
                }
            }
            _ => {}
        }
        false
    }
}
