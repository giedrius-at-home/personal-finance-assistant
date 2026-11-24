//! The headless Finance server (`main.rs`) and the pieces the devserver
//! shares with it: the command table and the small HTTP layer.

pub mod auth;
pub mod commands;
pub mod http;
