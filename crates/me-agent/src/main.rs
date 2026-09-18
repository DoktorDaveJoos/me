fn main() {
    let mut args = std::env::args().skip(1);
    let path = match args.next().as_deref() {
        None => me_agent::default_vault_path(),
        Some("--vault-dir") => args
            .next()
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute()),
        _ => None,
    };
    let Some(path) = path.filter(|_| args.next().is_none()) else {
        eprintln!("Usage: me-mcp [--vault-dir /absolute/vault]");
        std::process::exit(2);
    };
    if me_agent::run_mcp(std::io::stdin().lock(), std::io::stdout().lock(), &path).is_err() {
        eprintln!("ME MCP transport closed.");
        std::process::exit(1);
    }
}
