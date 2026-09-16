use agent_sandbox::*;
#[tokio::main]
async fn main() {
    let (backend, _) = detect_backend();
    let dir = tempfile::tempdir().unwrap();
    let cfg = SandboxConfig { workspace_dir: dir.path().to_path_buf(), allow_network: false, timeout_secs: 10, ..Default::default() };
    // try to read ~/.ssh — should fail or be empty inside bwrap
    let out = backend.run_command("ls ~/.ssh 2>&1; echo ---; cat ~/.ssh/id_rsa 2>&1 | head -2; echo ---; env | grep -iE 'key|token|secret' || echo 'env clean'", &[], &cfg).await.unwrap();
    println!("{}", out.stdout);
    println!("stderr: {}", out.stderr);
}
