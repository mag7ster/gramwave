fn main() -> std::process::ExitCode {
    if let Some(code) = gramwave::run_cli() {
        return code;
    }
    match gramwave::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
