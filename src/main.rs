use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use workboard_rs::App;

#[tokio::main(worker_threads = 2)]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("workboard-rs: {message}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "dev") {
        if args.len() != 2 {
            return Err("usage: workboard-rs dev PRIVATE_LAUNCH_JSON".into());
        }
        return workboard_rs::dev::launch(std::path::Path::new(&args[1]))
            .await
            .map_err(|e| e.to_string());
    }
    if args.first().is_some_and(|a| a == "ownership") {
        let value = workboard_rs::ownership::cli(&args[1..]).map_err(|e| e.to_string())?;
        println!("{value}");
        return Ok(());
    }
    if !args.is_empty() {
        if args.len() != 3 || (args[0] != "snapshot" && args[0] != "restore") {
            return Err(
                "usage: workboard-rs [snapshot SOURCE NEW_FILE | restore SNAPSHOT NEW_DB]".into(),
            );
        }
        workboard_rs::offline::copy(
            std::path::Path::new(&args[1]),
            std::path::Path::new(&args[2]),
        )
        .map_err(|e| e.to_string())?;
        eprintln!("workboard-rs: offline copy complete; execution remains paused");
        return Ok(());
    }
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "SIGTERM handler unavailable")?;
    let key = env::var("WORKBOARD_OPERATOR_KEY")
        .map_err(|_| "WORKBOARD_OPERATOR_KEY required (32–256 bytes)".to_string())?;
    let path = PathBuf::from(
        env::var_os("WORKBOARD_DB").ok_or("WORKBOARD_DB required (absolute isolated path)")?,
    );
    let port = env::var("WORKBOARD_PORT")
        .unwrap_or_else(|_| "4311".into())
        .parse::<u16>()
        .map_err(|_| "WORKBOARD_PORT invalid")?;
    if port == 4310 || port == 0 {
        return Err("WORKBOARD_PORT must be nonzero and separate from live port 4310".into());
    }
    let app = App::open_approved(
        &path,
        &key,
        port,
        workboard_rs::execution_config::Approval::from_env(),
    )
    .map_err(|e| e.to_string())?;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|_| "loopback listener unavailable")?;
    let engine = app.start_execution();
    eprintln!("workboard-rs listening at http://{address}; guarded candidate execution");
    let shutdown = app.clone();
    let result = axum::serve(listener, app.router())
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            shutdown.shutdown();
        })
        .await
        .map_err(|_| "HTTP server stopped unexpectedly".to_string());
    app.shutdown();
    let _ = engine.await;
    app.finish_ownership().map_err(|e| e.to_string())?;
    result
}
