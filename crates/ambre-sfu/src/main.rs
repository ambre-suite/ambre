//! The SFU binary. The environment sets it:
//!
//! | Variable | Holds |
//! | :--- | :--- |
//! | `SFU_SIGNAL_ADDR` | the TCP address of the signalling WebSocket |
//! | `SFU_MEDIA_ADDR` | the UDP address the browsers send their media to |
//! | `SFU_METRICS_ADDR` | the TCP address of the metrics, the loopback by default (`CALL-101`) |
//! | `SFU_METRICS_TOKEN` | the token a scrape presents; required off the loopback (`CALL-141`) |
//! | `SFU_ADMISSION_KEY` | the relay's public key, in hex, which signs every admission |
//! | `SFU_TURN_URLS` | the TURN servers, separated by commas; none when unset |
//! | `SFU_TURN_SECRET` | the secret the TURN server shares with the SFU alone (`CALL-014`) |

use std::net::SocketAddr;
use std::process::ExitCode;

use ambre_core::{FixedBytes, PublicKey, Validated};
use ambre_metrics::ScrapeAccess;
use ambre_sfu::admit::AdmissionKey;
use ambre_sfu::secrets::{TurnSecret, scrape_token};
use ambre_sfu::serve::{Addresses, Admitting, run};
use ambre_sfu::turn::{TurnServers, TurnUrl};

/// The two addresses the environment may set.
#[derive(Clone, Copy)]
enum Setting {
    Signal,
    Media,
    Metrics,
}

impl Setting {
    /// The variable that holds the address.
    const fn variable(self) -> &'static str {
        match self {
            Self::Signal => "SFU_SIGNAL_ADDR",
            Self::Media => "SFU_MEDIA_ADDR",
            Self::Metrics => "SFU_METRICS_ADDR",
        }
    }

    /// The address when the variable is unset: the loopback, for a local run.
    const fn fallback(self) -> SocketAddr {
        let port = match self {
            Self::Signal => 3480,
            Self::Media => 3481,
            Self::Metrics => 3482,
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

/// The relay's public key, which `SFU_ADMISSION_KEY` holds. The relay logs it at its start.
fn admission_key() -> Result<AdmissionKey, String> {
    let text = std::env::var("SFU_ADMISSION_KEY")
        .map_err(|_| "SFU_ADMISSION_KEY: the relay's public key is unset".to_owned())?;
    let key = FixedBytes::<PublicKey>::from_hex(text.trim())
        .map_err(|e| format!("SFU_ADMISSION_KEY: no key of 32 bytes in hex: {e}"))?;
    AdmissionKey::new(&key).map_err(|e| format!("SFU_ADMISSION_KEY: no Ed25519 key: {e}"))
}

/// The TURN servers, when `SFU_TURN_URLS` names any. The secret must come with them.
fn turn_servers() -> Result<Option<TurnServers>, String> {
    let Ok(urls) = std::env::var("SFU_TURN_URLS") else {
        return Ok(None);
    };
    let urls = urls
        .split(',')
        .map(|url| {
            Validated::<TurnUrl>::parse(url.trim()).map_err(|e| format!("SFU_TURN_URLS: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let secret = std::env::var("SFU_TURN_SECRET")
        .map(TurnSecret::new)
        .map_err(|_| "SFU_TURN_SECRET: the TURN servers come with their secret".to_owned())?;
    Ok(Some(TurnServers::new(urls, secret)))
}

#[tokio::main]
async fn main() -> ExitCode {
    let settings = (|| {
        let addresses = Addresses {
            signal: Setting::Signal.read()?,
            media: Setting::Media.read()?,
            metrics: Setting::Metrics.read()?,
        };
        let scrape = ScrapeAccess::for_listener(addresses.metrics, scrape_token())
            .map_err(|e| format!("SFU_METRICS_TOKEN: {e}"))?;
        let admitting = Admitting {
            key: admission_key()?,
            turn: turn_servers()?,
            scrape,
        };
        Ok::<_, String>((addresses, admitting))
    })();
    let (addresses, admitting) = match settings {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(addresses, admitting).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("the SFU stopped: {e}");
            ExitCode::FAILURE
        }
    }
}
