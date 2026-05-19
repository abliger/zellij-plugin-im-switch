/// WASM 插件接口层。
///
/// Zellij 通过调用这些 C 风格的入口函数（#[no_mangle]）与插件交互。
/// 每个函数从 stdin 读取 protobuf 数据，转换为 Rust 类型后，
/// 委托给 State 上对应的方法处理。
use std::cell::RefCell;
use std::collections::BTreeMap;
use zellij_tile::prelude::*;

// Zellij 以裸函数形式调用入口点，不会传入 State 实例。
// 我们将单例存放在 thread_local 中，每个钩子都从这里访问。
thread_local! {
    static STATE: RefCell<crate::State> = RefCell::new(Default::default());
}

/// WASM 要求的入口点（此插件无实际操作）。
#[no_mangle]
pub fn _start() {}

/// WASM 要求的入口点（此插件无实际操作）。
#[no_mangle]
pub fn __main_void() -> i32 {
    0
}

/// 插件加载时调用一次。接收来自 Zellij 布局的用户配置，
/// 并转发给 State::load 处理。
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

/// 每次 Zellij 事件触发时调用（标签切换、pane 焦点变化、命令结果等）。
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

/// 其他进程通过 zellij pipe 向此插件发送消息时调用。
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
            eprintln!("pipe: failed to decode pipe message");
            return false;
        };
        let Ok(pipe_message) = protobuf_pipe_message.try_into() else {
            eprintln!("pipe: failed to convert pipe message");
            return false;
        };
        state.borrow_mut().pipe(pipe_message)
    })
}

/// Zellij 要求插件渲染 UI 时调用（此插件无 UI）。
#[no_mangle]
pub fn render(rows: i32, cols: i32) {
    STATE.with(|state| {
        state.borrow_mut().render(rows as usize, cols as usize);
    });
}

/// Zellij 在 UI 中显示插件版本时调用。
#[no_mangle]
pub fn plugin_version() {
    println!("{}", crate::VERSION);
}
