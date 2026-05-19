use std::cell::RefCell;
use std::collections::BTreeMap;
use zellij_tile::prelude::*;

thread_local! {
    static STATE: RefCell<crate::State> = RefCell::new(Default::default());
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

#[no_mangle]
pub fn render(rows: i32, cols: i32) {
    STATE.with(|state| {
        state.borrow_mut().render(rows as usize, cols as usize);
    });
}

#[no_mangle]
pub fn plugin_version() {
    println!("{}", crate::VERSION);
}
