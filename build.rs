fn main() {
    println!("cargo:rerun-if-env-changed=WEBSKIFF_BUILD_COMMIT");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    let commit = std::env::var("WEBSKIFF_BUILD_COMMIT").unwrap_or_else(|_| {
        std::process::Command::new("git")
            .args(["rev-parse", "--short=12", "HEAD"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|| "unknown".into())
    });
    assert!(
        commit == "unknown"
            || (commit.len() >= 7
                && commit.len() <= 40
                && commit.bytes().all(|c| c.is_ascii_hexdigit())),
        "WEBSKIFF_BUILD_COMMIT must be a Git commit hash"
    );
    println!("cargo:rustc-env=WEBSKIFF_BUILD_COMMIT={commit}");
    println!(
        "cargo:rustc-env=WEBSKIFF_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    zebkit_build::bundle();
}
