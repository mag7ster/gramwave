use crate::tr;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProcessIdentity {
    pub pid: u32,
    start_time: u64,
}

impl ProcessIdentity {
    pub fn current() -> Result<Self, String> {
        let pid = std::process::id();
        let (start_time, _) = stats(pid)
            .map_err(|error| format!("{} {error}", tr("Cannot identify the player process.")))?;
        Ok(Self { pid, start_time })
    }

    pub fn is_running(self) -> Result<bool, String> {
        match stats(self.pid) {
            Ok((start_time, state)) => {
                Ok(start_time == self.start_time && !matches!(state, 'Z' | 'X' | 'x'))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!(
                "{} {error}",
                tr("Cannot check the player process.")
            )),
        }
    }
}

fn stats(pid: u32) -> io::Result<(u64, char)> {
    let contents = std::fs::read(format!("/proc/{pid}/stat"))?;
    // The process name can contain parentheses and spaces.
    let contents = String::from_utf8_lossy(&contents);
    let fields = (|| {
        let mut fields = contents.rsplit_once(") ")?.1.split_ascii_whitespace();
        let state = fields.next()?.chars().next()?;
        let start_time = fields.nth(18)?.parse().ok()?;
        Some((start_time, state))
    })();
    fields.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid process stat"))
}
