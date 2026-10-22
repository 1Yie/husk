//! Which shell commands are plain workspace observation.
//!
//! A command counts only when every stage is a known read, search or listing
//! (or a neutral text filter between them) and every path it names stays
//! inside the workspace. Anything else — a redirect, a substitution, a write
//! flag, an unknown program, a path outside the root or in a tree the file
//! tools skip — is left unclassified and the command runs without a remark. A
//! missed classification costs one hint; a wrong one would lecture a
//! legitimate command, so every rule errs toward silence.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObservationKind {
    Read,
    Search,
    List,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ShellObservation {
    /// What the command did, deduplicated and ordered.
    pub kinds: Vec<ObservationKind>,
    /// The one existing workspace file a pure read targets — set only when
    /// the command reads exactly one file and does nothing else.
    pub file: Option<PathBuf>,
}

/// Trees `.gitignore` normally hides from `smart_grep` / `list_dir`; searching
/// or listing inside them is a legitimate `bash` job.
const SKIPPED_TREES: [&str; 6] = ["node_modules", "target", ".git", "dist", "build", "vendor"];

/// Classify `command` as run from `root`. `None` means "not plain observation".
pub fn shell_observation(command: &str, root: &Path) -> Option<ShellObservation> {
    let root = normalize(root);
    let mut cwd = root.clone();
    let mut kinds = Vec::new();
    let mut files = Vec::new();
    let mut stages = 0usize;
    for stage in stages_of(command)? {
        let (program, args) = stage.split_first()?;
        let program = program.rsplit('/').next().unwrap_or(program);
        if program == "cd" {
            let dir = args.first()?;
            cwd = inside(&cwd, dir, &root)?;
            continue;
        }
        stages += 1;
        let kind = match program {
            "cat" | "head" | "nl" | "wc" => Some(ObservationKind::Read),
            "tail"
                if !args
                    .iter()
                    .any(|a| a == "-f" || a == "-F" || a.starts_with("--follow")) =>
            {
                Some(ObservationKind::Read)
            }
            "sed" if sed_is_read(args) => Some(ObservationKind::Read),
            "grep" | "egrep" | "fgrep" | "rg" => Some(ObservationKind::Search),
            "find" if !args.iter().any(|a| FIND_ACTIONS.contains(&a.as_str())) => {
                Some(ObservationKind::Search)
            }
            "ls" | "tree" => Some(ObservationKind::List),
            "echo" | "printf" | "sort" | "uniq" | "cut" | "tr" | "column" | "pwd" | "true" => None,
            _ => return None,
        };
        let mut named = false;
        for arg in args.iter().filter(|a| !a.starts_with('-')) {
            let path = inside(&cwd, arg, &root)?;
            if kind.is_some_and(|k| k != ObservationKind::Read) && in_skipped_tree(&path, &root) {
                return None;
            }
            if path.exists() {
                named = true;
                if kind == Some(ObservationKind::Read) && path.is_file() {
                    files.push(path);
                }
            }
        }
        // A search or listing with no path runs over the current directory.
        if !named
            && kind.is_some_and(|k| k != ObservationKind::Read)
            && in_skipped_tree(&cwd, &root)
        {
            return None;
        }
        // A read program with no file is a filter on the previous stage.
        match kind {
            Some(ObservationKind::Read) if !named => {}
            Some(k) => kinds.push(k),
            None => {}
        }
    }
    if kinds.is_empty() {
        return None;
    }
    let pure_read = stages == 1 && kinds == [ObservationKind::Read];
    kinds.sort();
    kinds.dedup();
    let file = (pure_read && files.len() == 1).then(|| files.remove(0));
    Some(ShellObservation { kinds, file })
}

const FIND_ACTIONS: [&str; 9] = [
    "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf", "-fls",
];

/// `sed` that only prints: `-n` present, no in-place flag, and a script that
/// neither writes nor runs anything. `-n` alone is not enough — `s/x/y/w out`
/// and `1e touch out` both write under it.
fn sed_is_read(args: &[String]) -> bool {
    let short = |a: &&String| a.starts_with('-') && !a.starts_with("--");
    let quiet = args.iter().filter(short).any(|a| a.contains('n'))
        || args.iter().any(|a| a == "--quiet" || a == "--silent");
    // `-i` edits in place; `-f` takes the script from a file, whose contents
    // are not in this command line to judge.
    let writes = args.iter().filter(short).any(|a| a.contains('i'))
        || args.iter().any(|a| a.starts_with("--in-place"))
        || args.iter().any(|a| a == "-f" || a.starts_with("--file"));
    if !quiet || writes {
        return false;
    }
    // The script is the first non-flag argument (or `-e`'s value). With no
    // script at all there is nothing to print.
    let mut script: Option<&str> = None;
    let mut skip_next = false;
    for a in args {
        if skip_next {
            script = Some(a);
            skip_next = false;
            continue;
        }
        if a == "-e" || a == "--expression" {
            skip_next = true;
            continue;
        }
        if let Some(rest) = a.strip_prefix("--expression=") {
            script = Some(rest);
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        script = Some(a);
        break;
    }
    script.is_some_and(sed_script_is_read)
}

/// Does this `sed` script only read? Commands are separated by `;` and
/// newlines: one that starts with `w`/`W` writes a file, `e` runs a shell
/// command, and `r`/`R` read a file named on the command line (harmless to the
/// workspace, so they stay allowed only through the separator rule below).
/// A `s` command's flags are checked too — `s/x/y/w out` writes.
fn sed_script_is_read(script: &str) -> bool {
    for command in script.split([';', '\n']) {
        let command = command.trim_start();
        let Some(first) = command.chars().next() else {
            continue;
        };
        if matches!(first, 'w' | 'W' | 'e') {
            return false;
        }
        // A line address in front of the command (`1w out`, `$e cmd`) must
        // not hide which command follows it.
        let after_address =
            command.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '$' | ','));
        if after_address.starts_with(['w', 'W', 'e']) {
            return false;
        }
        if first == 's' && !substitute_flags_are_read(command) {
            return false;
        }
    }
    true
}

/// The flags after an `s` command's third delimiter. `w` writes the pattern
/// space to a file, `e` executes it; both are writes in everything but name.
fn substitute_flags_are_read(command: &str) -> bool {
    let mut chars = command.char_indices();
    chars.next(); // the `s` itself
    let Some((_, delimiter)) = chars.next() else {
        return true;
    };
    let mut delimiters: Vec<usize> = Vec::new();
    let mut escaped = false;
    for (at, ch) in chars {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == delimiter {
            delimiters.push(at);
            if delimiters.len() == 2 {
                break;
            }
        }
    }
    // Unterminated `s` command: it cannot run, and it certainly cannot be
    // relied on to be a read.
    let Some(&third) = delimiters.get(1) else {
        return false;
    };
    let flags = &command[third + delimiter.len_utf8()..];
    !flags.chars().any(|f| matches!(f, 'w' | 'W' | 'e'))
}

/// Resolve `arg` against `cwd`, lexically. `None` when it leaves `root` or
/// names a home directory. Unquoted variables never reach here — the splitter
/// refuses them — so a `$` left in a word is a quoted regex anchor.
fn inside(cwd: &Path, arg: &str, root: &Path) -> Option<PathBuf> {
    if arg.starts_with('~') {
        return None;
    }
    // Glob characters: judge by the literal prefix before the first one.
    let literal = arg.split(['*', '?', '[', '{']).next().unwrap_or("");
    let joined = if Path::new(literal).is_absolute() {
        PathBuf::from(literal)
    } else {
        cwd.join(literal)
    };
    let path = normalize(&joined);
    path.starts_with(root).then_some(path)
}

fn in_skipped_tree(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|rel| {
        rel.components()
            .any(|c| SKIPPED_TREES.iter().any(|t| c.as_os_str() == *t))
    })
}

/// Lexical `.` / `..` folding — no filesystem access, no symlink resolution.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Split a command into stages of words at `&&`, `||`, `;`, `|` and newlines.
/// `None` for anything that is not a plain pipeline of simple commands:
/// redirects (other than discarding stderr), command or process substitution,
/// variable expansion, background jobs, unbalanced quotes.
fn stages_of(command: &str) -> Option<Vec<Vec<String>>> {
    let mut stages = vec![Vec::new()];
    let mut word = String::new();
    let mut quoted = false; // the current word had quotes, so `""` is a word
    let mut chars = command.chars().peekable();
    let end_word = |word: &mut String, quoted: &mut bool, stages: &mut Vec<Vec<String>>| {
        if !word.is_empty() || *quoted {
            stages.last_mut().unwrap().push(std::mem::take(word));
        }
        *quoted = false;
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                quoted = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        ch => word.push(ch),
                    }
                }
            }
            '"' => {
                quoted = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        '`' => return None,
                        '$' if chars
                            .peek()
                            .is_some_and(|n| n.is_alphabetic() || matches!(n, '{' | '(' | '_')) =>
                        {
                            return None
                        }
                        ch => word.push(ch),
                    }
                }
            }
            '\\' => word.push(chars.next()?),
            '`' | '<' | '(' | ')' => return None,
            '$' if chars
                .peek()
                .is_some_and(|n| n.is_alphabetic() || matches!(n, '{' | '(' | '_')) =>
            {
                return None
            }
            '>' => {
                // Only `2>/dev/null` and `2>&1` — discarding or merging stderr.
                if word != "2" {
                    return None;
                }
                word.clear();
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
                let rest: String = chars.clone().take(9).collect();
                let skip = if rest.starts_with("/dev/null") {
                    9
                } else if rest.starts_with("&1") {
                    2
                } else {
                    return None;
                };
                for _ in 0..skip {
                    chars.next();
                }
            }
            '&' => {
                // A lone `&` backgrounds a job — not a plain pipeline.
                chars.next_if_eq(&'&')?;
                end_word(&mut word, &mut quoted, &mut stages);
                stages.push(Vec::new());
            }
            '|' | ';' | '\n' => {
                chars.next_if_eq(&'|');
                end_word(&mut word, &mut quoted, &mut stages);
                stages.push(Vec::new());
            }
            c if c.is_whitespace() => end_word(&mut word, &mut quoted, &mut stages),
            c => word.push(c),
        }
    }
    end_word(&mut word, &mut quoted, &mut stages);
    Some(stages.into_iter().filter(|s| !s.is_empty()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ObservationKind::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        dir
    }

    fn kinds(cmd: &str, root: &Path) -> Option<Vec<ObservationKind>> {
        shell_observation(cmd, root).map(|o| o.kinds)
    }

    #[test]
    fn plain_reads_searches_and_listings_are_observation() {
        let ws = workspace();
        let root = ws.path();
        for (cmd, want) in [
            ("cat src/lib.rs", vec![Read]),
            ("sed -n '10,40p' src/lib.rs", vec![Read]),
            ("head -50 src/lib.rs", vec![Read]),
            ("grep -rn \"fn main\" src", vec![Search]),
            ("rg -n needle", vec![Search]),
            ("ls -la src", vec![List]),
            ("find src -name '*.rs'", vec![Search]),
            (
                "sed -n '1,5p' src/lib.rs && echo --- && grep -n main src/main.rs",
                vec![Read, Search],
            ),
            ("grep -rn x src 2>/dev/null | head -30", vec![Search]),
            ("grep -n 'x$' src/lib.rs", vec![Search]),
        ] {
            assert_eq!(kinds(cmd, root), Some(want), "{cmd}");
        }
        // `cd <workspace> && …` — the shape models emit most.
        let cmd = format!("cd {} && sed -n '1,3p' src/lib.rs", root.display());
        assert_eq!(kinds(&cmd, root), Some(vec![Read]));
    }

    #[test]
    fn commands_that_run_or_write_are_not_observation() {
        let ws = workspace();
        let root = ws.path();
        for cmd in [
            "cargo test",
            "git diff src/lib.rs",
            "git log --oneline | head",
            "npm run build",
            "sed -i 's/a/b/' src/lib.rs",
            "sed -ni 's/a/b/p' src/lib.rs",
            // `-n` alone does not make `sed` a read: these write a file or run
            // a command despite it.
            "sed -n 's/a/b/w src/out.txt' src/lib.rs",
            "sed -n 's/a/b/e' src/lib.rs",
            "sed -n '1e touch pwned' src/lib.rs",
            "sed -n 'w src/out.txt' src/lib.rs",
            "sed -n -f src/script.sed src/lib.rs",
            "find . -name '*.tmp' -fprint0 hits.bin",
            "cat > src/new.rs <<'EOF'\nfn x() {}\nEOF",
            "echo hi > src/x.txt",
            "find . -name '*.tmp' -delete",
            "tail -f log.txt",
            "python3 -c 'print(1)'",
            "grep -n x $(git ls-files)",
            "cat `ls`",
            "echo $HOME",
            "for f in src/*.rs; do wc -l $f; done",
            "pwd",
        ] {
            assert_eq!(kinds(cmd, root), None, "{cmd}");
        }
    }

    /// Outside the workspace and inside the trees `smart_grep` skips, `bash`
    /// is the right tool — no hint.
    #[test]
    fn targets_the_file_tools_cannot_reach_are_not_flagged() {
        let ws = workspace();
        let root = ws.path();
        for cmd in [
            "sed -n '1,40p' ~/.cargo/registry/src/x/lib.rs",
            "cat /etc/hosts",
            "grep -rn foo ../other-repo",
            "grep -rn needle node_modules/pkg",
            "cd node_modules && grep -rn needle .",
            "ls /usr/lib",
        ] {
            assert_eq!(kinds(cmd, root), None, "{cmd}");
        }
        // Reading one file under a skipped tree is still a plain read.
        std::fs::write(root.join("node_modules/pkg/index.js"), "x\n").unwrap();
        assert_eq!(
            kinds("cat node_modules/pkg/index.js", root),
            Some(vec![Read])
        );
    }

    #[test]
    fn a_single_file_read_names_its_file() {
        let ws = workspace();
        let root = ws.path();
        let o = shell_observation("sed -n '1,2p' src/lib.rs", root).unwrap();
        assert_eq!(o.file, Some(normalize(&root.join("src/lib.rs"))));
        let two = shell_observation("cat src/lib.rs src/main.rs", root).unwrap();
        assert_eq!(two.file, None);
        let piped = shell_observation("cat src/lib.rs | grep a", root).unwrap();
        assert_eq!(piped.file, None, "a pipeline is not a plain read");
    }
}
