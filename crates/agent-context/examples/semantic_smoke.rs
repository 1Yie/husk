//! End-to-end check of the semantic encoder: fetch the real model, load it,
//! and score a cross-language pair. Run with
//! `cargo run -p agent-context --features semantic --example semantic_smoke`.

use std::time::Duration;

use agent_context::memory::semantic::{self, Phase};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

fn main() {
    if !semantic::available() {
        eprintln!("build without the `semantic` feature — rerun with --features semantic");
        return;
    }
    let dir = std::env::temp_dir().join("husk-semantic-smoke");
    std::fs::create_dir_all(&dir).unwrap();
    semantic::set_enabled(true, dir);

    let mut last = String::new();
    loop {
        let p = semantic::progress();
        let line = format!(
            "{:?} {} / {} {}",
            p.phase,
            p.bytes,
            p.total,
            p.file.as_deref().unwrap_or("")
        );
        if line != last {
            eprintln!("{line}");
            last = line;
        }
        match p.phase {
            Phase::Ready => break,
            Phase::Failed => {
                eprintln!("FAILED: {}", p.error.unwrap_or_default());
                std::process::exit(1);
            }
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }

    let zh = semantic::embed("用中文回答").expect("embed zh");
    let en = semantic::embed("answer in Chinese").expect("embed en");
    let other = semantic::embed("先跑 fmt 再 test").expect("embed other");
    eprintln!("dim = {}", zh.len());
    eprintln!("zh/en   = {:.3}", cosine(&zh, &en));
    eprintln!("zh/other= {:.3}", cosine(&zh, &other));
    assert!(cosine(&zh, &en) > 0.5, "cross-language pair should be close");
    eprintln!("OK");
}
