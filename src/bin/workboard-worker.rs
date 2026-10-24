#[tokio::main(worker_threads = 2)]
async fn main() {
    let result = if let Some(path) = std::env::args_os()
        .nth(1)
        .filter(|v| v == "--native-job")
        .and_then(|_| std::env::args_os().nth(2))
    {
        workboard_rs::adapters::native::job_entry(std::path::Path::new(&path)).await
    } else {
        workboard_rs::adapters::main_entry("worker").await
    };
    if let Err(e) = result {
        e.report_failure();
        std::process::exit(1);
    }
}
