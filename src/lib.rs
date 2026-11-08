pub mod adapters;
pub mod application;
pub mod domain;
use adapters::{audio, discovery, media_loader, storage, streaming, telegram};
use domain::{model, playback};
#[cfg(test)]
mod artwork_tests;
mod auth;
mod bootstrap;
mod design;
mod i18n;
mod screens;
mod ui;
use i18n::tr;
mod demo;
use application::controller::{App, Event, RetryAction};
use model::{Playlist, State, Track};
use playback::RepeatMode;

pub use adapters::control::run_cli;
pub use bootstrap::run;
