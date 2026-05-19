use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use zellij_tile::prelude::*;

mod shim;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
pub struct State {
    active_tab: Option<usize>,
    focused_pane: Option<PaneId>,
    im_select: String,
    state_dir: String,
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace("'", "'\\''"))
}

fn clear_old_state(dir: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("ime") {
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("clear_old_state: failed to remove {:?}: {}", path, e);
            }
        }
    }
}

impl State {
    fn log(&self, msg: &str) {
        use std::time::{SystemTime, UNIX_EPOCH};

        let log_path = format!("{}/debug.log", self.state_dir);
        let mut file = match OpenOptions::new().create(true).append(true).open(&log_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("log: failed to open {}: {}", log_path, e);
                return;
            }
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let ts = format!(
            "{:02}:{:02}:{:02}",
            (secs / 3600) % 24,
            (secs / 60) % 60,
            secs % 60
        );

        if let Err(e) = writeln!(file, "[{}] {}", ts, msg) {
            eprintln!("log: failed to write: {}", e);
        }
    }

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

    fn switch_ime(&self, old_id: Option<PaneId>, new_id: PaneId) {
        self.log(&format!("pane switch: old={:?}, new={:?}", old_id, new_id));

        let mut ctx = BTreeMap::new();
        ctx.insert("im_switch".to_string(), "1".to_string());

        match old_id {
            Some(old) => {
                let im = shell_quote(&self.im_select);
                let dir = shell_quote(&self.state_dir);
                let script = format!(
                    "mkdir -p {dir}; \
                     OLD=$({im}); \
                     printf '%s' \"$OLD\" > {dir}/\"$1\".ime; \
                     if [ -f {dir}/\"$2\".ime ]; then \
                         NEW=$(cat {dir}/\"$2\".ime); \
                         [ \"$OLD\" != \"$NEW\" ] && {im} \"$NEW\"; \
                     fi",
                );
                run_command(
                    &["sh", "-c", &script, "--", &file_name(&old), &file_name(&new_id)],
                    ctx,
                );
            }
            None => {
                let im = shell_quote(&self.im_select);
                let dir = shell_quote(&self.state_dir);
                let script = format!(
                    "mkdir -p {dir}; \
                     if [ -f {dir}/\"$1\".ime ]; then \
                         {im} \"$(cat {dir}/\"$1\".ime)\"; \
                     fi",
                );
                run_command(
                    &["sh", "-c", &script, "--", &file_name(&new_id)],
                    ctx,
                );
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
        clear_old_state(&self.state_dir);

        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::RunCommandResult,
        ]);
        self.log("plugin loaded");
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(_) => {}
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
                let old_id = self.focused_pane;
                self.focused_pane = Some(new_id);
                self.switch_ime(old_id, new_id);
            }
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

fn file_name(id: &PaneId) -> String {
    match id {
        PaneId::Terminal(n) => format!("t{}", n),
        PaneId::Plugin(n) => format!("p{}", n),
    }
}
