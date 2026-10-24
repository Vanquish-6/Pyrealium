//! Build 7148 protocol laboratory and local emulator.
//! Native compatibility is established per codec, not inferred from simulated tests.
pub mod bootstrap;
pub mod content;
pub mod crypto;
pub mod dat;
pub mod inventory;
pub mod inventory_request;
pub mod legacy;
pub mod movement;
pub mod preferences;
pub mod progression;
pub mod scene;
pub mod script;
pub mod server;
pub mod state;
pub mod store;
pub mod transport;
pub mod ui;
pub mod wire;
pub mod world;
