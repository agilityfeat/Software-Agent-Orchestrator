#[tokio::main(worker_threads = 2)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = if args.len() == 3 && args[1] == "--watchdog" {
        workboard_rs::adapters::contained::watchdog(&args[2]).await
    } else if args.len() == 2 && args[1] == "--validate-profile" {
        workboard_rs::adapters::contained::validate_profile().await
    } else if args.len() == 2 && args[1] == "--stop-worker" {
        workboard_rs::adapters::contained::stop_worker().await
    } else if args.len() == 2
        && ["--export-evidence", "--export-patch", "--export-auth"].contains(&args[1].as_str())
    {
        workboard_rs::adapters::contained::export_artifact(&args[1])
    } else if args.len() == 1 {
        workboard_rs::adapters::contained::main_entry().await
    } else {
        std::process::exit(1)
    };
    if let Err(e) = result {
        e.report_failure();
        std::process::exit(1);
    }
}
