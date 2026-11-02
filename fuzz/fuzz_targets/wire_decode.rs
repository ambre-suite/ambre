//! Any bytes a peer or the relay sends decode to a message or to a typed
//! error, on both sides of the socket.
#![no_main]

use ambre_core::Payload;
use ambre_protocol::ws::WireFrame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let frame = Payload::<WireFrame>::new(data.to_vec());
    let _ = ambre_protocol::ws::decode_client_msg(&frame);
    let _ = ambre_protocol::ws::decode_server_msg(&frame);
});
