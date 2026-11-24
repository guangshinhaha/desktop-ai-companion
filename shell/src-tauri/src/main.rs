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
    // `companion stats`: weekly replay returns and chat returns, from the local usage log.
    // `companion stats --export <file>` writes the per-day counts to a file the officer names.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("stats") {
        use companion_shell::usage::{default_path, UsageLog};
        let run = || -> Result<(), String> {
            let path = default_path().ok_or("no home directory")?;
            let log = UsageLog::open(&path)?;
            if args.get(1).map(String::as_str) == Some("--export") {
                let dest = args.get(2).ok_or("usage: stats --export <file>")?;
                return log.export_to(std::path::Path::new(dest));
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs() as i64;
            print!("{}", log.stats(now, 4)?);
            Ok(())
        };
        if let Err(e) = run() {
            eprintln!("stats failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    companion_shell::run();
}
