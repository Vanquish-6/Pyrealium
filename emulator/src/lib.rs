//! Build 7148 protocol laboratory and local emulator.
//! Native compatibility is established per codec, not inferred from simulated tests.
pub mod bootstrap;
pub mod crypto;
pub mod dat;
pub mod server;
pub mod state;
pub mod store;
pub mod transport;
pub mod wire;
pub mod world;
