pub mod auth;
pub mod commands;
pub mod core;
pub mod error;
pub mod legacy;
pub mod library;
pub mod media;
pub mod model;
pub mod notice;
pub mod offline;
pub mod persistence;
pub mod playback;
pub mod ports;
pub mod tasks;
pub mod telegram;
pub mod workflow;

mod library_workflow;
mod offline_work;

pub use core::AppCore;
pub use error::AppError;
