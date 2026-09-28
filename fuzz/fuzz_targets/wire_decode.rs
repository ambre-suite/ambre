//! Any bytes a peer or the relay sends decode to a message or to a typed
//! error, on both sides of the socket.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ambre_protocol::ws::decode_client_msg(data);
    let _ = ambre_protocol::ws::decode_server_msg(data);
});
