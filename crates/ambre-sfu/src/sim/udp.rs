//! The impairment relay over real UDP: one link per participant, in front of the SFU.
//!
//! A participant sends to the front socket of its link. The link applies the impairment of
//! the uplink and sends from its back socket to the SFU, which sees the back socket as the
//! participant. What the SFU answers takes the impairment of the downlink, then leaves the
//! front socket towards the participant. A delay is fixed, so each direction stays in order.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use super::impair::Impairment;

/// One running link.
pub struct Link {
    /// The address a participant sends to.
    pub front: SocketAddr,
    /// The impairment of the downlink, which a test may cap while the link runs.
    pub down: Arc<Mutex<Impairment>>,
}

/// A datagram held until it leaves.
type Held = (Instant, Vec<u8>);

/// Starts a link to the SFU at `sfu` on the current runtime.
pub async fn link(sfu: SocketAddr, up: Impairment, down: Impairment) -> std::io::Result<Link> {
    let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let back = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let front_addr = front.local_addr()?;
    let up = Arc::new(Mutex::new(up));
    let down = Arc::new(Mutex::new(down));
    let (client_tx, client_rx) = tokio::sync::watch::channel(None::<SocketAddr>);

    let (up_tx, up_rx) = mpsc::unbounded_channel::<Held>();
    tokio::spawn(admit(front.clone(), up, up_tx, Some(client_tx)));
    tokio::spawn(release(back.clone(), up_rx, Target::Fixed(sfu)));

    let (down_tx, down_rx) = mpsc::unbounded_channel::<Held>();
    tokio::spawn(admit(back, down.clone(), down_tx, None));
    tokio::spawn(release(front, down_rx, Target::Watched(client_rx)));

    Ok(Link {
        front: front_addr,
        down,
    })
}

/// Where a released datagram goes.
enum Target {
    Fixed(SocketAddr),
    /// The last address the participant sent from.
    Watched(tokio::sync::watch::Receiver<Option<SocketAddr>>),
}

/// Reads `socket`, and holds each datagram the impairment keeps until its time.
async fn admit(
    socket: Arc<UdpSocket>,
    impairment: Arc<Mutex<Impairment>>,
    out: mpsc::UnboundedSender<Held>,
    client: Option<tokio::sync::watch::Sender<Option<SocketAddr>>>,
) {
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok((n, source)) = socket.recv_from(&mut buf).await else {
            return;
        };
        if let Some(client) = &client
            && *client.borrow() != Some(source)
        {
            let _ = client.send(Some(source));
        }
        let now = Instant::now();
        let at = impairment.lock().ok().and_then(|mut i| i.admit(now, n));
        if let (Some(at), Some(bytes)) = (at, buf.get(..n))
            && out.send((at, bytes.to_vec())).is_err()
        {
            return;
        }
    }
}

/// Sends each held datagram at its time.
async fn release(socket: Arc<UdpSocket>, mut held: mpsc::UnboundedReceiver<Held>, to: Target) {
    while let Some((at, bytes)) = held.recv().await {
        tokio::time::sleep_until(at.into()).await;
        let destination = match &to {
            Target::Fixed(addr) => Some(*addr),
            Target::Watched(rx) => *rx.borrow(),
        };
        if let Some(addr) = destination {
            let _ = socket.send_to(&bytes, addr).await;
        }
    }
}
