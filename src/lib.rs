use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use zellij_tile::prelude::*;

const VERSION: &str = "0.1.0";

#[derive(Default)]
struct State {
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
                if self.active_tab.is_none() {
                    for (tab_pos, panes) in &manifest.panes {
                        if panes.iter().any(|p| p.is_focused) {
                            self.active_tab = Some(*tab_pos);
                            break;
                        }
                    }
                }
                let tab = match self.active_tab {
                    Some(t) => t,
                    None => return false,
                };
                let panes = match manifest.panes.get(&tab) {
                    Some(p) => p,
                    None => return false,
                };

                let p = panes
                    .iter()
                    .find(|p| p.is_focused && !p.is_plugin && p.is_floating)
                    .or_else(|| panes.iter().find(|p| p.is_focused && !p.is_plugin));

                let p = match p {
                    Some(p) => p,
                    None => return false,
                };

                let new_id = pane_id(p);
                if self.focused_pane == Some(new_id) {
                    return false;
                }
                let old_id = self.focused_pane;
                self.focused_pane = Some(new_id);
                self.log(&format!("pane switch: old={:?}, new={:?}", old_id, new_id));

                let mut ctx = BTreeMap::new();
                ctx.insert("im_switch".to_string(), "1".to_string());

                match old_id {
                    Some(old) => {
                        let script = format!(
                            "mkdir -p '{dir}'; \
                             OLD=$({im}); \
                             printf '%s' \"$OLD\" > '{dir}/'\"$1\".ime; \
                             if [ -f '{dir}/'\"$2\".ime ]; then \
                                 NEW=$(cat '{dir}/'\"$2\".ime); \
                                 [ \"$OLD\" != \"$NEW\" ] && {im} \"$NEW\"; \
                             fi",
                            im = self.im_select,
                            dir = self.state_dir,
                        );
                        run_command(
                            &["sh", "-c", &script, "--", &file_name(&old), &file_name(&new_id)],
                            ctx,
                        );
                    }
                    None => {
                        let script = format!(
                            "mkdir -p '{dir}'; \
                             if [ -f '{dir}/'\"$1\".ime ]; then \
                                 {im} \"$(cat '{dir}/'\"$1\".ime)\"; \
                             fi",
                            im = self.im_select,
                            dir = self.state_dir,
                        );
                        run_command(
                            &["sh", "-c", &script, "--", &file_name(&new_id)],
                            ctx,
                        );
                    }
                }
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

fn pane_id(p: &PaneInfo) -> PaneId {
    debug_assert!(!p.is_plugin);
    PaneId::Terminal(p.id)
}

fn file_name(id: &PaneId) -> String {
    match id {
        PaneId::Terminal(n) => format!("t{}", n),
        PaneId::Plugin(n) => format!("p{}", n),
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(Default::default());
}

#[no_mangle]
pub fn _start() {}

#[no_mangle]
pub fn __main_void() -> i32 {
    0
}

#[no_mangle]
pub fn load() {
    STATE.with(|state| {
        use std::convert::TryFrom;
        use zellij_tile::shim::plugin_api::action::ProtobufPluginConfiguration;
        use zellij_tile::shim::prost::Message;

        let Ok(protobuf_bytes) = zellij_tile::shim::object_from_stdin::<Vec<u8>>() else {
            eprintln!("load: failed to read stdin");
            return;
        };
        let Ok(protobuf_configuration) =
            ProtobufPluginConfiguration::decode(protobuf_bytes.as_slice())
        else {
            eprintln!("load: failed to decode protobuf");
            return;
        };
        let Ok(plugin_configuration) = BTreeMap::try_from(&protobuf_configuration) else {
            eprintln!("load: failed to convert configuration");
            return;
        };
        state.borrow_mut().load(plugin_configuration);
    });
}

#[no_mangle]
pub fn update() -> bool {
    STATE.with(|state| {
        use std::convert::TryInto;
        use zellij_tile::shim::plugin_api::event::ProtobufEvent;
        use zellij_tile::shim::prost::Message;

        let Ok(protobuf_bytes) = zellij_tile::shim::object_from_stdin::<Vec<u8>>() else {
            eprintln!("update: failed to read stdin");
            return false;
        };
        let Ok(protobuf_event) = ProtobufEvent::decode(protobuf_bytes.as_slice()) else {
            eprintln!("update: failed to decode protobuf");
            return false;
        };
        let Ok(event) = protobuf_event.try_into() else {
            eprintln!("update: failed to convert event");
            return false;
        };
        state.borrow_mut().update(event)
    })
}

#[no_mangle]
pub fn pipe() -> bool {
    STATE.with(|state| {
        use std::convert::TryInto;
        use zellij_tile::shim::plugin_api::pipe_message::ProtobufPipeMessage;
        use zellij_tile::shim::prost::Message;

        let Ok(protobuf_bytes) = zellij_tile::shim::object_from_stdin::<Vec<u8>>() else {
            eprintln!("pipe: failed to read stdin");
            return false;
        };
        let Ok(protobuf_pipe_message) = ProtobufPipeMessage::decode(protobuf_bytes.as_slice())
        else {
            eprintln!("pipe: failed to decode protobuf");
            return false;
        };
        let Ok(pipe_message) = protobuf_pipe_message.try_into() else {
            eprintln!("pipe: failed to convert pipe message");
            return false;
        };
        state.borrow_mut().pipe(pipe_message)
    })
}

#[no_mangle]
pub fn render(rows: i32, cols: i32) {
    STATE.with(|state| {
        state.borrow_mut().render(rows as usize, cols as usize);
    });
}

#[no_mangle]
pub fn plugin_version() {
    println!("{}", VERSION);
}
