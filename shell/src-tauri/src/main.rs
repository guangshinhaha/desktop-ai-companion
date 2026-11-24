fn main() {
    // Headless check for the sidecar link: `companion-shell --ping-executor` starts the executor,
    // pings it over stdio, prints the result and exits. No window, no tray.
    if std::env::args().any(|a| a == "--ping-executor") {
        match companion_shell::executor::ping(&companion_shell::executor::executor_path()) {
            Ok(v) => println!("executor ping ok: pong, version {v}"),
            Err(e) => {
                eprintln!("executor ping failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    companion_shell::run();
}
