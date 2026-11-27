use crate::{adapters::control, identity::APP_ID, tr};
use std::{
    env,
    fs::{DirBuilder, OpenOptions},
    io,
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn launch(demo: bool) -> Result<(), String> {
    let slot = control::slot(demo);
    if control::instance(&slot).is_ok() {
        println!("{}", tr("The player is already running."));
        return Ok(());
    }
    let log_path = log_path(demo)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&log_path)
        .map_err(|error| log_error(&log_path, error))?;
    log.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| log_error(&log_path, error))?;
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new(executable);
    command
        .args(env::args_os().skip(1).filter(|arg| arg != "--daemon"))
        .stdin(Stdio::null())
        .stdout(
            log.try_clone()
                .map_err(|error| log_error(&log_path, error))?,
        )
        .stderr(log);
    // setsid is async-signal-safe and disconnects the child from the terminal.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| log_error(&log_path, error))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| log_error(&log_path, error))?
        {
            if status.success() && control::instance(&slot).is_ok() {
                println!("{}", tr("The player is already running."));
                return Ok(());
            }
            return Err(log_error(&log_path, status));
        }
        if control::instance(&slot).is_ok_and(|identity| identity.pid == child.id()) {
            println!(
                "{} {}",
                tr("Player started. Log file at"),
                log_path.display()
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Only the child launched above can be stopped here.
            let _ = child.kill();
            let _ = child.wait();
            return Err(log_error(
                &log_path,
                tr("Timed out waiting for the player to start"),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn log_path(demo: bool) -> Result<PathBuf, String> {
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| tr("No user state directory for the daemon log"))?;
    let directory = state.join(APP_ID);
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .map_err(|error| log_error(&directory, error))?;
    Ok(directory.join(if demo {
        "daemon-demo.log"
    } else {
        "daemon.log"
    }))
}

fn log_error(path: &std::path::Path, error: impl std::fmt::Display) -> String {
    format!(
        "{} {error}. {} {}",
        tr("Cannot start the player in the background."),
        tr("Log file at"),
        path.display()
    )
}
