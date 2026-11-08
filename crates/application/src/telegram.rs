use crate::{
    error::AppError,
    media::{MediaPlaylist, MediaTrack},
};
use std::{future::Future, pin::Pin};
use tg_player_domain::{MediaKind, TrackId};
use tokio_util::sync::CancellationToken;
pub type GatewayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AuthorizationState {
    #[default]
    Starting,
    Parameters,
    Phone,
    Confirmation(String),
    Code {
        email: bool,
        delivery: String,
        next: bool,
        timeout: u64,
    },
    Password(String),
    Email,
    Ready,
    Closed,
}
#[derive(Clone, Debug)]
pub struct TelegramParameters {
    pub api_id: i32,
    pub api_hash: String,
    pub database: String,
    pub files: String,
}
#[derive(Clone, Debug)]
pub enum AuthRequest {
    Qr,
    Phone(String),
    Code(String),
    EmailCode(String),
    Password(String),
    Email(String),
    Resend,
    Parameters(TelegramParameters),
    Close,
}
#[derive(Clone, Debug, Default)]
pub struct TelegramFile {
    pub id: i64,
    pub size: u64,
    pub downloaded: u64,
    pub path: Option<String>,
    pub complete: bool,
}
pub enum TelegramUpdate {
    Authorization(AuthorizationState),
    File(TelegramFile),
    Chat(MediaPlaylist),
    ChatTitle(i64, String),
    Message {
        id: TrackId,
        track: Option<MediaTrack>,
    },
    Deleted {
        chat: i64,
        messages: Vec<i64>,
        from_cache: bool,
    },
}
#[derive(Debug)]
pub struct MediaPage {
    /// Highest server message ID before playable-media filtering.
    pub newest: i64,
    pub tracks: Vec<MediaTrack>,
    pub next: i64,
}
pub trait TelegramGateway: Send + Sync {
    fn search_media<'a>(
        &'a self,
        chat: i64,
        kind: MediaKind,
        cursor: i64,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, MediaPage>;
    fn message<'a>(
        &'a self,
        id: TrackId,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, MediaTrack>;
    fn file<'a>(
        &'a self,
        id: i64,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, TelegramFile>;
    fn authenticate<'a>(
        &'a self,
        request: AuthRequest,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, ()>;
}
impl AuthorizationState {
    pub fn delivery(&self) -> &str {
        if let Self::Code { delivery, .. } = self {
            delivery
        } else {
            ""
        }
    }
    pub fn email_code(&self) -> bool {
        matches!(self, Self::Code { email: true, .. })
    }
    pub fn can_resend(&self) -> bool {
        matches!(self, Self::Code { next: true, .. })
    }
    pub fn password_hint(&self) -> &str {
        if let Self::Password(hint) = self {
            hint
        } else {
            ""
        }
    }
}
