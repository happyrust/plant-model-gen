fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=.git/HEAD");
    
    if let Ok(output) = std::process::Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if output.status.success() {
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=GIT_COMMIT={}", commit);
        }
    }
    
    let build_date = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    println!("cargo:rustc-env=BUILD_DATE={}", build_date);
    
    Ok(())
}
