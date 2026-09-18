use clap::Parser;

fn main() -> std::process::ExitCode {
    #[cfg(unix)]
    {
        let args: Vec<_> = std::env::args_os().collect();
        if args
            .get(1)
            .is_some_and(|a| a == orochi::process::SUPERVISE_FLAG)
        {
            return orochi::process::supervise(&args[2..]);
        }
        if args
            .get(1)
            .is_some_and(|a| a == orochi::mailbox::SERVE_FLAG)
        {
            return orochi::mailbox::serve(&args[2..]);
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("orochi: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match runtime.block_on(orochi::cli::execute(orochi::cli::Cli::parse())) {
        Ok(code) => std::process::ExitCode::from(code),
        Err(error) => {
            eprintln!("orochi: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
