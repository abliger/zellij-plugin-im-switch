use std::cell::RefCell;
use std::collections::BTreeMap;
use zellij_tile::prelude::*;

const IM_SELECT: &str = "/Users/lvming/.local/bin/im-select";
const STATE_DIR: &str = "/Users/lvming/.cache/zellij-ime";
const LOG_FILE: &str = "/Users/lvming/.cache/zellij-ime/debug.log";

#[derive(Default)]
struct State {
    active_tab: Option<usize>,
    focused_pane: Option<PaneId>,
}

fn log(msg: &str) {
    let script = format!("mkdir -p '{STATE_DIR}' && echo '[$(date +%H:%M:%S)] {msg}' >> '{LOG_FILE}'");
    run_command(&["sh", "-c", &script], BTreeMap::new());
}

impl ZellijPlugin for State {
    fn load(&mut self, _config: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
        ]);
        subscribe(&[EventType::TabUpdate, EventType::PaneUpdate, EventType::RunCommandResult]);
        log("plugin loaded");
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(permission_status) => {
                match permission_status {
                    PermissionStatus::Granted => {
                        log("permissions granted");
                    }
                    PermissionStatus::Denied => {
                        log("permissions DENIED");
                    }
                }
            }
            Event::TabUpdate(tabs) => {
                log(&format!("TabUpdate: {} tabs", tabs.len()));
                if let Some(t) = tabs.iter().find(|t| t.active) {
                    self.active_tab = Some(t.position);
                    log(&format!("  active_tab={}", t.position));
                }
            }
            Event::PaneUpdate(manifest) => {
                let tab = match self.active_tab {
                    Some(t) => t,
                    None => {
                        log("PaneUpdate: active_tab is None, skipping");
                        return false;
                    }
                };
                let panes = match manifest.panes.get(&tab) {
                    Some(p) => p,
                    None => {
                        log(&format!("PaneUpdate: no panes for tab {tab}"));
                        return false;
                    }
                };

                let p = panes
                    .iter()
                    .find(|p| p.is_focused && !p.is_plugin && p.is_floating)
                    .or_else(|| panes.iter().find(|p| p.is_focused && !p.is_plugin));

                let p = match p {
                    Some(p) => p,
                    None => {
                        log(&format!(
                            "PaneUpdate: no focused non-plugin pane in tab {tab}, panes={}",
                            panes.len()
                        ));
                        return false;
                    }
                };

                let new_id = pane_id(p);
                if self.focused_pane == Some(new_id) {
                    return false;
                }
                let old_id = self.focused_pane;
                self.focused_pane = Some(new_id);
                log(&format!("pane switch: old={:?}, new={:?}", old_id, new_id));

                let script = match old_id {
                    Some(old) => format!(
                        "mkdir -p {dir}; \
                         OLD=$({im}); \
                         printf '%s' \"$OLD\" > {dir}/{old}.ime; \
                         if [ -f {dir}/{new}.ime ]; then \
                             NEW=$(cat {dir}/{new}.ime); \
                             [ \"$OLD\" != \"$NEW\" ] && {im} \"$NEW\"; \
                         fi",
                        im = IM_SELECT,
                        dir = STATE_DIR,
                        old = old_file_name(&old),
                        new = new_file_name(&new_id)
                    ),
                    None => format!(
                        "mkdir -p {dir}; \
                         if [ -f {dir}/{new}.ime ]; then \
                             {im} \"$(cat {dir}/{new}.ime)\"; \
                         fi",
                        im = IM_SELECT,
                        dir = STATE_DIR,
                        new = new_file_name(&new_id)
                    ),
                };
                run_command(&["sh", "-c", &script], BTreeMap::new());
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                log(&format!("RunCommandResult: exit={:?} ctx={:?} stdout={} stderr={}",
                    exit_code, context, String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr)));
            }
            _ => {}
        }
        false
    }
}

fn pane_id(p: &PaneInfo) -> PaneId {
    if p.is_plugin {
        PaneId::Plugin(p.id)
    } else {
        PaneId::Terminal(p.id)
    }
}

fn old_file_name(id: &PaneId) -> String {
    match id {
        PaneId::Terminal(n) => format!("t{}", n),
        PaneId::Plugin(n) => format!("p{}", n),
    }
}

fn new_file_name(id: &PaneId) -> String {
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
        use std::convert::TryInto;
        use zellij_tile::shim::plugin_api::action::ProtobufPluginConfiguration;
        use zellij_tile::shim::prost::Message;
        let protobuf_bytes: Vec<u8> = zellij_tile::shim::object_from_stdin().unwrap();
        let protobuf_configuration: ProtobufPluginConfiguration =
            ProtobufPluginConfiguration::decode(protobuf_bytes.as_slice()).unwrap();
        let plugin_configuration: BTreeMap<String, String> =
            BTreeMap::try_from(&protobuf_configuration).unwrap();
        state.borrow_mut().load(plugin_configuration);
    });
}

#[no_mangle]
pub fn update() -> bool {
    use std::convert::TryInto;
    use zellij_tile::shim::plugin_api::event::ProtobufEvent;
    use zellij_tile::shim::prost::Message;
    STATE.with(|state| {
        let protobuf_bytes: Vec<u8> = zellij_tile::shim::object_from_stdin().unwrap();
        let protobuf_event: ProtobufEvent =
            ProtobufEvent::decode(protobuf_bytes.as_slice()).unwrap();
        let event = protobuf_event.try_into().unwrap();
        state.borrow_mut().update(event)
    })
}

#[no_mangle]
pub fn pipe() -> bool {
    use std::convert::TryInto;
    use zellij_tile::shim::plugin_api::pipe_message::ProtobufPipeMessage;
    use zellij_tile::shim::prost::Message;
    STATE.with(|state| {
        let protobuf_bytes: Vec<u8> = zellij_tile::shim::object_from_stdin().unwrap();
        let protobuf_pipe_message: ProtobufPipeMessage =
            ProtobufPipeMessage::decode(protobuf_bytes.as_slice()).unwrap();
        let pipe_message = protobuf_pipe_message.try_into().unwrap();
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
    println!("{}", zellij_tile::prelude::VERSION);
}
