//! `cargo run -p agent-context --example blueprint_check [root]` — print the
//! blueprint index, derived graph and rule check for a workspace.

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let root = std::path::Path::new(&root);
    let Some(bp) = agent_context::Blueprint::load(root) else {
        eprintln!("no .blueprint/ under {}", root.display());
        std::process::exit(2);
    };
    print!("{}", bp.index_block());
    println!();
    print!("{}", bp.graph().mermaid(&bp.nodes));
    println!();
    let report = bp.check();
    print!("{}", report.render());
    std::process::exit(if report.errors() > 0 { 1 } else { 0 });
}
