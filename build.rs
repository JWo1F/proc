use std::process::Command;

fn main() {
  // Only build frontend when the "web" feature is enabled
  if std::env::var("CARGO_FEATURE_WEB").is_err() {
    return;
  }

  let frontend_dir = std::path::Path::new("frontend");

  // Rerun if frontend sources change
  println!("cargo:rerun-if-changed=frontend/src");
  println!("cargo:rerun-if-changed=frontend/index.html");
  println!("cargo:rerun-if-changed=frontend/vite.config.js");
  println!("cargo:rerun-if-changed=frontend/package.json");

  // Install dependencies if node_modules doesn't exist
  if !frontend_dir.join("node_modules").exists() {
    let status = Command::new("pnpm")
      .arg("install")
      .current_dir(frontend_dir)
      .status()
      .expect("Failed to run pnpm install. Is pnpm available?");
    assert!(status.success(), "pnpm install failed");
  }

  // Build the frontend
  let status = Command::new("pnpm")
    .args(["build"])
    .current_dir(frontend_dir)
    .status()
    .expect("Failed to run pnpm build. Is pnpm available?");
  assert!(status.success(), "pnpm build failed");
}
