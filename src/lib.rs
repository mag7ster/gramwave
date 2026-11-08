pub mod adapters;
pub mod application;
use adapters::{discovery, media_loader, streaming, telegram};
use tg_player_application::model;
pub use tg_player_domain as domain;
use tg_player_domain::playback;
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
pub mod launch;
use application::controller::{DesktopApp as App, Event, RetryAction};
use model::{Playlist, State};
mod presentation;
use playback::RepeatMode;
use presentation::Track;

pub use adapters::control::run_cli;
pub use bootstrap::run;
