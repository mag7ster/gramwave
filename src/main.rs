fn main() -> std::process::ExitCode {
    if let Some(code) = tg_player::run_cli() {
        return code;
    }
    match tg_player::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
