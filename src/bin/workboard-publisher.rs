#[tokio::main(worker_threads = 2)]
async fn main() {
    if let Some(prompt) = std::env::args().nth(1) {
        if let Ok(repo) = std::env::var("WORKBOARD_GIT_REPOSITORY") {
            if prompt.trim_end() == format!("Username for 'https://github.com/{repo}.git':") {
                println!("x-access-token");
                return;
            }
            if prompt.trim_end()
                == format!("Password for 'https://x-access-token@github.com/{repo}.git':")
            {
                if let Ok(token) = std::env::var("GH_TOKEN") {
                    println!("{token}");
                    return;
                }
            }
        }
        std::process::exit(1);
    }
    if let Err(e) = workboard_rs::adapters::main_entry("publisher").await {
        e.report_failure();
        std::process::exit(1);
    }
}
