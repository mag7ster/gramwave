//! Native service construction lives at the desktop composition boundary.
use crate::{
    adapters::{audio, storage, streaming, telegram},
    application::ports::{PlayerBackend, Repository},
    launch::LaunchOptions,
    tr,
};
use std::{path::PathBuf, sync::mpsc};
use tg_player_application::{tasks::Scheduler, telegram::TelegramUpdate, AppCore};

pub(crate) struct Services {
    pub root: PathBuf,
    pub core: AppCore,
    pub repository: Option<Box<dyn Repository>>,
    pub player: Option<Box<dyn PlayerBackend>>,
    pub player_error: String,
    pub tasks: Scheduler,
    pub media: Option<fastframe_now_playing::NowPlaying>,
    pub tray: Option<fastframe_tray::Tray>,
    pub demo: bool,
}
impl Services {
    pub(crate) fn for_launch(options: &LaunchOptions) -> Result<Self, String> {
        let root = if options.demo {
            PathBuf::new()
        } else {
            data_root()
        };
        Self::open(root, options.demo, options.desktop_integrations)
    }
    pub(crate) fn open(
        root: PathBuf,
        demo: bool,
        desktop_integrations: bool,
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
            let (store, state) = storage::Store::open(&root.join("library.sqlite"))?;
            let offline = store.offline();
            (Some(Box::new(store) as Box<dyn Repository>), state, offline)
        };
        let (player, player_error) = if demo {
            (None, String::new())
        } else {
            match audio::Player::new() {
                Ok(player) => (
                    Some(Box::new(player) as Box<dyn PlayerBackend>),
                    String::new(),
                ),
                Err(error) => (None, error),
            }
        };
        let desktop = !demo || desktop_integrations;
        let media = desktop.then(|| {
            fastframe_now_playing::NowPlaying::start(
                fastframe_now_playing::App::new("tgplayer", "TG Player"),
                || {},
            )
        });
        let tray = if desktop {
            fastframe_tray::Tray::spawn(
                fastframe_tray::Config {
                    id: "tgplayer",
                    title: "TG Player".into(),
                    icon: crate::application::controller::tray_icon,
                    template_icon: None,
                    themed_icon: false,
                    menu: vec![
                        fastframe_tray::MenuItem::action("show", tr("Show player")),
                        fastframe_tray::MenuItem::action("play", tr("Play / pause")),
                        fastframe_tray::MenuItem::action("next", tr("Next")),
                        fastframe_tray::MenuItem::Separator,
                        fastframe_tray::MenuItem::action("quit", tr("Quit")),
                    ],
                },
                || {},
            )
        } else {
            None
        };
        Ok(Self {
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
pub(crate) fn connect(handle: tokio::runtime::Handle) -> Result<Connection, String> {
    let (telegram, updates) = telegram::Telegram::start()?;
    telegram.offline();
    telegram.configure_cache();
    let streaming = streaming::serve(telegram.clone(), handle);
    Ok(Connection {
        telegram,
        updates,
        streaming,
    })
}
