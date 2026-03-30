fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=RELEASE_VERSION");

    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        && output.status.success()
    {
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!("cargo:rustc-env=GIT_COMMIT={}", commit);
    }

    let build_date = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d %H:%M:%S UTC+8")
        .to_string();
    println!("cargo:rustc-env=BUILD_DATE={}", build_date);

    let app_version = std::env::var("RELEASE_VERSION")
        .ok()
        .map(|value| value.trim().trim_start_matches('v').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string())
        });
    println!("cargo:rustc-env=APP_VERSION={}", app_version);

    Ok(())
}
