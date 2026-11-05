#![forbid(unsafe_code)]

#[tokio::main]
async fn main() -> std::process::ExitCode {
    ab_runtime::cli::main().await
}
