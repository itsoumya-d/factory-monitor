//! api-server: HTTP + WebSocket surface of the engine (docs/10).
//! Stored data shape = docs/11; repo trait decouples routes from storage.

pub mod agg;
pub mod config;
pub mod db;
pub mod error;
pub mod hub;
pub mod repo;
pub mod routes;
pub mod state;
pub mod webhook;
pub mod ws;
