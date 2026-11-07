//! The SFU binary. `SFU_SIGNAL_ADDR` names the TCP address of the signalling WebSocket, and
//! `SFU_MEDIA_ADDR` the UDP address the browsers send their media to.

use std::net::SocketAddr;
use std::process::ExitCode;

use ambre_sfu::serve::{Addresses, run};

/// The two addresses the environment may set.
#[derive(Clone, Copy)]
enum Setting {
    Signal,
    Media,
}

impl Setting {
    /// The variable that holds the address.
    const fn variable(self) -> &'static str {
        match self {
            Self::Signal => "SFU_SIGNAL_ADDR",
            Self::Media => "SFU_MEDIA_ADDR",
        }
    }

    /// The address when the variable is unset: the loopback, for a local run.
    const fn fallback(self) -> SocketAddr {
        let port = match self {
            Self::Signal => 3480,
            Self::Media => 3481,
        };
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port)
    }

    /// The address the environment sets, or the fallback when it sets none.
    fn read(self) -> Result<SocketAddr, String> {
        match std::env::var(self.variable()) {
            Err(_) => Ok(self.fallback()),
            Ok(text) => text
                .parse()
                .map_err(|e| format!("{}: {text:?} is no socket address: {e}", self.variable())),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let addresses = match (Setting::Signal.read(), Setting::Media.read()) {
        (Ok(signal), Ok(media)) => Addresses { signal, media },
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(addresses).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("the SFU stopped: {e}");
            ExitCode::FAILURE
        }
    }
}
