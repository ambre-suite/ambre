//! The simulation of a call: the impairment of a link, synthetic participants, and the
//! network that joins them to the SFU core.

mod client;
mod impair;
mod load;
mod net;
mod udp;

#[cfg(test)]
mod tests;
