//! Native service construction lives at the desktop composition boundary.
use crate::{
    adapters::{storage, streaming, telegram, tray::Tray},
    application::ports::{PlayerBackend, Repository},
    launch::LaunchOptions,
};
use fastframe_shell::Waker;
use std::{path::PathBuf, sync::mpsc};
use tg_player_application::{tasks::Scheduler, telegram::TelegramUpdate, AppCore};

pub(crate) struct Services {
    pub wake: Waker,
    pub root: PathBuf,
    pub core: AppCore,
    pub repository: Option<Box<dyn Repository>>,
    pub player: Option<Box<dyn PlayerBackend>>,
    pub player_error: String,
    pub tasks: Scheduler,
    pub media: Option<fastframe_now_playing::NowPlaying>,
    pub tray: Option<Tray>,
    pub demo: bool,
}
impl Services {
    pub(crate) fn for_launch(options: &LaunchOptions, wake: Waker) -> Result<Self, String> {
        let root = if options.demo {
            PathBuf::new()
        } else {
            data_root()
        };
        Self::open(root, options.demo, options.desktop_integrations, wake)
    }
    pub(crate) fn open(
        root: PathBuf,
        demo: bool,
        desktop_integrations: bool,
        wake: Waker,
    ) -> Result<Self, String> {
        let (repository, state, offline) = if demo {
            (None, crate::demo::state(), Default::default())
        } else {
            std::fs::create_dir_all(&root).map_err(|e| format!("create app data: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).ok();
            }
            let (store, state) =
                storage::Store::open_with_wake(&root.join("library.sqlite"), wake.clone())?;
            let mut offline = store.offline();
            crate::adapters::offline::recover_metadata(&root.join("offline"), &mut offline);
            (Some(Box::new(store) as Box<dyn Repository>), state, offline)
        };
        let (player, player_error) = if demo {
            (None, String::new())
        } else {
            #[cfg(feature = "mpv")]
            match crate::adapters::audio::Player::new(wake.clone()) {
                Ok(player) => (
                    Some(Box::new(player) as Box<dyn PlayerBackend>),
                    String::new(),
                ),
                Err(error) => (None, error),
            }
            #[cfg(not(feature = "mpv"))]
            (
                None,
                crate::tr("This build does not include audio playback"),
            )
        };
        let desktop = !demo || desktop_integrations;
        let media = desktop.then(|| {
            let wake = wake.clone();
            fastframe_now_playing::NowPlaying::start(
                fastframe_now_playing::App::new("tgplayer", "Gramwave"),
                move || wake.wake(),
            )
        });
        let tray = if desktop {
            Tray::spawn(wake.clone())
        } else {
            None
        };
        Ok(Self {
            wake,
            root,
            core: AppCore::with_offline(state, offline),
            repository,
            player,
            player_error,
            tasks: Scheduler::new(),
            media,
            tray,
            demo,
        })
    }
}
fn data_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("tg-player")
}

pub(crate) struct Connection {
    pub telegram: telegram::Telegram,
    pub updates: mpsc::Receiver<TelegramUpdate>,
    pub streaming: Result<(String, streaming::Server), String>,
}
/// Connect lazily after credentials are available, using the one desktop runtime.
pub(crate) fn connect(handle: tokio::runtime::Handle, wake: Waker) -> Result<Connection, String> {
    let (telegram, updates) = telegram::Telegram::start(wake)?;
    telegram.offline();
    telegram.configure_cache();
    let streaming = streaming::serve(telegram.clone(), handle);
    Ok(Connection {
        telegram,
        updates,
        streaming,
    })
}
