// smoke check: run both scanners on this repo
fn main() {
    let root = std::env::args().nth(1).unwrap();
    let tree = agent_context::WorkspaceScanner::build_skeleton(&root).unwrap();
    println!("entries: {} truncated: {}", tree.entries.len(), tree.truncated);
    for e in tree.entries.iter().take(15) { println!("  {e}"); }
    let snap = agent_context::git_snapshot(&root).unwrap();
    print!("{}", snap.to_prompt_block());
}
