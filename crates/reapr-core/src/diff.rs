//! Changed line ranges from `git diff --unified=0`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::FileRange;

#[derive(thiserror::Error, Debug)]
pub enum DiffError {
    #[error("failed to run `{command}`: {source}")]
    Io {
        command: String,
        source: std::io::Error,
    },
    #[error("`{command}` failed (exit code {code:?}): {stderr}")]
    Git {
        command: String,
        code: Option<i32>,
        stderr: String,
    },
}

/// Parses a `--unified=0` diff into added/changed line ranges in the new files, relative to the git
/// root. Ranges are merged when they overlap or touch, and sorted by path then start.
pub fn parse_unified_zero(diff: &str) -> Vec<FileRange> {
    let mut ranges = Vec::new();
    let mut file: Option<PathBuf> = None;
    // Lines still expected in the current hunk, so content like `+++ x` is never read as a header.
    let (mut old_left, mut new_left) = (0u32, 0u32);

    for line in diff.lines() {
        if old_left > 0 || new_left > 0 {
            match line.as_bytes().first() {
                Some(b'-') if old_left > 0 => {
                    old_left -= 1;
                    continue;
                }
                Some(b'+') if new_left > 0 => {
                    new_left -= 1;
                    continue;
                }
                Some(b'\\') => continue,
                _ => (old_left, new_left) = (0, 0),
            }
        }

        if line.starts_with("diff --git ") {
            file = None;
        } else if let Some(raw) = line.strip_prefix("+++ ") {
            file = parse_path(raw);
        } else if let Some((old_count, start, count)) = parse_hunk(line) {
            (old_left, new_left) = (old_count, count);
            if let Some(path) = &file
                && count > 0
            {
                ranges.push(FileRange {
                    path: path.clone(),
                    start,
                    end: start.saturating_add(count - 1),
                });
            }
        }
    }

    ranges.sort_by(|a, b| (&a.path, a.start).cmp(&(&b.path, b.start)));
    let mut merged: Vec<FileRange> = Vec::with_capacity(ranges.len());
    for r in ranges {
        if let Some(last) = merged.last_mut()
            && last.path == r.path
            && r.start <= last.end.saturating_add(1)
        {
            last.end = last.end.max(r.end);
        } else {
            merged.push(r);
        }
    }
    merged
}

/// `@@ -a[,b] +c[,d] @@` → `(b, c, d)`, with a missing count meaning 1.
fn parse_hunk(line: &str) -> Option<(u32, u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let range = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (_, old_count) = range(old)?;
    let (start, count) = range(new)?;
    Some((old_count, start, count))
}

fn parse_path(raw: &str) -> Option<PathBuf> {
    if raw == "/dev/null" {
        return None;
    }
    let path = match raw.strip_prefix('"') {
        Some(quoted) => unquote(quoted)?,
        // Git appends a tab to unquoted names that contain spaces.
        None => raw.strip_suffix('\t').unwrap_or(raw).to_owned(),
    };
    Some(PathBuf::from(path.strip_prefix("b/").unwrap_or(&path)))
}

/// Undoes git's C-style quoting; `s` starts right after the opening quote.
fn unquote(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        match b {
            b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
            b'\\' => {
                let e = bytes.next()?;
                out.push(match e {
                    b'0'..=b'7' => {
                        let mut v = u32::from(e - b'0');
                        for _ in 0..2 {
                            let d = bytes.next().filter(|d| (b'0'..=b'7').contains(d))?;
                            v = v * 8 + u32::from(d - b'0');
                        }
                        u8::try_from(v).ok()?
                    }
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'v' => 0x0b,
                    other => other,
                });
            }
            _ => out.push(b),
        }
    }
    None
}

/// Line ranges changed between the merge base of `base` and `HEAD`, relative to `project_dir`.
/// Files outside `project_dir` are dropped.
pub async fn changed_ranges(project_dir: &Path, base: &str) -> Result<Vec<FileRange>, DiffError> {
    let rev = git(
        project_dir,
        &["rev-parse", "--show-toplevel", "--show-prefix"],
    )
    .await?;
    let mut lines = rev.lines();
    let root = lines.next().unwrap_or_default();
    let prefix = Path::new(lines.next().unwrap_or_default());

    let range = format!("{base}...HEAD");
    let diff = git(
        Path::new(root),
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            // The user's `diff.interHunkContext` fuses nearby hunks even with --unified=0, and a
            // textconv filter changes line numbers: both would shift the scope.
            "--inter-hunk-context=0",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            // `base` is a revision even when it starts with `-` (e.g. `--output=<path>`).
            "--end-of-options",
            &range,
            "--",
        ],
    )
    .await?;

    Ok(parse_unified_zero(&diff)
        .into_iter()
        .filter_map(|r| {
            let path = r.path.strip_prefix(prefix).ok()?.to_path_buf();
            Some(FileRange { path, ..r })
        })
        .collect())
}

pub(crate) async fn git(cwd: &Path, args: &[&str]) -> Result<String, DiffError> {
    let command = format!("git {}", args.join(" "));
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|source| DiffError::Io {
            command: command.clone(),
            source,
        })?;
    if !out.status.success() {
        return Err(DiffError::Git {
            command,
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(path: &str, start: u32, end: u32) -> FileRange {
        FileRange {
            path: path.into(),
            start,
            end,
        }
    }

    const MULTI: &str = "\
diff --git a/src/a.ts b/src/a.ts
index 1111111..2222222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -3 +3 @@ function f() {
-old
+new
@@ -10,0 +11,2 @@
+x
+y
diff --git a/src/b.ts b/src/b.ts
index 3333333..4444444 100644
--- a/src/b.ts
+++ b/src/b.ts
@@ -1,2 +1,3 @@
-a
-b
+a
+b
+c
";

    #[test]
    fn multiple_files_and_hunks() {
        assert_eq!(
            parse_unified_zero(MULTI),
            vec![
                r("src/a.ts", 3, 3),
                r("src/a.ts", 11, 12),
                r("src/b.ts", 1, 3)
            ]
        );
    }

    #[test]
    fn crlf_input() {
        let crlf = MULTI.replace('\n', "\r\n");
        assert_eq!(parse_unified_zero(&crlf), parse_unified_zero(MULTI));
    }

    #[test]
    fn missing_count_means_one_line() {
        let d = "+++ b/a.ts\n@@ -7 +9 @@\n-x\n+y\n";
        assert_eq!(parse_unified_zero(d), vec![r("a.ts", 9, 9)]);
    }

    #[test]
    fn pure_deletion_contributes_nothing() {
        let d = "--- a/a.ts\n+++ b/a.ts\n@@ -5,2 +4,0 @@\n-x\n-y\n@@ -20 +18 @@\n-p\n+q\n";
        assert_eq!(parse_unified_zero(d), vec![r("a.ts", 18, 18)]);
    }

    #[test]
    fn deleted_file_contributes_nothing() {
        let d = "\
diff --git a/gone.ts b/gone.ts
deleted file mode 100644
index abcdef0..0000000
--- a/gone.ts
+++ /dev/null
@@ -1,2 +0,0 @@
-a
-b
";
        assert_eq!(parse_unified_zero(d), vec![]);
    }

    #[test]
    fn new_file() {
        let d = "\
diff --git a/new.ts b/new.ts
new file mode 100644
index 0000000..abcdef0
--- /dev/null
+++ b/new.ts
@@ -0,0 +1,3 @@
+a
+b
+c
";
        assert_eq!(parse_unified_zero(d), vec![r("new.ts", 1, 3)]);
    }

    #[test]
    fn renames_binaries_and_mode_changes() {
        let d = "\
diff --git a/old.ts b/renamed.ts
similarity index 90%
rename from old.ts
rename to renamed.ts
index 1111111..2222222 100644
--- a/old.ts
+++ b/renamed.ts
@@ -2 +2 @@
-x
+y
diff --git a/r1.ts b/r2.ts
similarity index 100%
rename from r1.ts
rename to r2.ts
diff --git a/img.png b/img.png
index 1111111..2222222 100644
Binary files a/img.png and b/img.png differ
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
";
        assert_eq!(parse_unified_zero(d), vec![r("renamed.ts", 2, 2)]);
    }

    #[test]
    fn quoted_and_spaced_paths() {
        let d = "\
+++ \"b/sp\\303\\251c ial.ts\"
@@ -0,0 +1 @@
+a
+++ \"b/q\\\"t\\\\x\\ty.ts\"
@@ -0,0 +2 @@
+a
+++ b/sp ace.ts\t
@@ -0,0 +3 @@
+a
";
        assert_eq!(
            parse_unified_zero(d),
            vec![
                r("q\"t\\x\ty.ts", 2, 2),
                r("sp ace.ts", 3, 3),
                r("spéc ial.ts", 1, 1),
            ]
        );
    }

    #[test]
    fn merges_adjacent_and_overlapping_ranges() {
        let d = "\
+++ b/a.ts
@@ -30 +30,5 @@
-x
+1
+2
+3
+4
+5
@@ -1 +1,2 @@
-x
+1
+2
@@ -2 +3 @@
-x
+1
@@ -40 +32,10 @@
-x
+1
+2
+3
+4
+5
+6
+7
+8
+9
+10
@@ -50 +50 @@
-x
+1
";
        assert_eq!(
            parse_unified_zero(d),
            vec![r("a.ts", 1, 3), r("a.ts", 30, 41), r("a.ts", 50, 50)]
        );
    }

    #[test]
    fn hunk_content_resembling_headers_is_ignored() {
        // Added lines `++ b/fake.ts` and `diff --git a/x b/x`, then a later hunk of the same file:
        // read as headers, they would move that hunk to `fake.ts` or drop it.
        let d = "+++ b/a.patch\n@@ -1 +1,2 @@\n-x\n+++ b/fake.ts\n+diff --git a/x b/x\n@@ -5 +6 @@\n-p\n+q\n";
        assert_eq!(
            parse_unified_zero(d),
            vec![r("a.patch", 1, 2), r("a.patch", 6, 6)]
        );
    }

    #[test]
    fn header_like_content_does_not_move_later_hunks_to_another_file() {
        // The added line `++ b/fake.ts` follows a removed line, then a second hunk in the same file.
        let d = "+++ b/a.patch\n@@ -1 +1 @@\n-x\n+++ b/fake.ts\n@@ -5 +5 @@\n-p\n+q\n";
        assert_eq!(
            parse_unified_zero(d),
            vec![r("a.patch", 1, 1), r("a.patch", 5, 5)]
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn changed_ranges_against_real_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let sub = root.join("packages/app");
        std::fs::create_dir_all(sub.join("src")).unwrap();
        std::fs::write(root.join("top.ts"), "1\n2\n3\n").unwrap();
        std::fs::write(sub.join("src/x.ts"), "a\nb\nc\nd\n").unwrap();
        git(root, &["init", "-q"]);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "one"]);

        std::fs::write(root.join("top.ts"), "1\nTWO\n3\n").unwrap();
        std::fs::write(sub.join("src/x.ts"), "a\nB\nC\nd\ne\n").unwrap();
        std::fs::write(sub.join("src/new.ts"), "n\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "two"]);

        assert_eq!(
            changed_ranges(root, "HEAD~1").await.unwrap(),
            vec![
                r("packages/app/src/new.ts", 1, 1),
                r("packages/app/src/x.ts", 2, 3),
                r("packages/app/src/x.ts", 5, 5),
                r("top.ts", 2, 2),
            ]
        );
        assert_eq!(
            changed_ranges(&sub, "HEAD~1").await.unwrap(),
            vec![
                r("src/new.ts", 1, 1),
                r("src/x.ts", 2, 3),
                r("src/x.ts", 5, 5)
            ]
        );
        assert!(matches!(
            changed_ranges(root, "no-such-ref").await,
            Err(DiffError::Git { .. })
        ));
    }

    /// Two commits: `before` (12 lines), then `after` applied to the same file.
    fn repo_with_change(before: &str, after: &str) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.ts"), before).unwrap();
        git(root, &["init", "-q"]);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "one"]);
        std::fs::write(root.join("a.ts"), after).unwrap();
        git(root, &["commit", "-q", "-am", "two"]);
        tmp
    }

    #[tokio::test]
    async fn user_inter_hunk_context_does_not_fuse_hunks() {
        let before: String = (1..=12).map(|i| format!("{i}\n")).collect();
        let after = before.replace("3\n", "THREE\n").replace("10\n", "TEN\n");
        let tmp = repo_with_change(&before, &after);
        git(tmp.path(), &["config", "diff.interHunkContext", "10"]);
        assert_eq!(
            changed_ranges(tmp.path(), "HEAD~1").await.unwrap(),
            vec![r("a.ts", 3, 3), r("a.ts", 10, 10)]
        );
    }

    #[tokio::test]
    async fn base_starting_with_dash_is_a_revision_not_an_option() {
        let tmp = repo_with_change("a\n", "b\n");
        let out = tempfile::tempdir().unwrap();
        let base = format!("--output={}", out.path().join("x").display());
        assert!(matches!(
            changed_ranges(tmp.path(), &base).await,
            Err(DiffError::Git { .. })
        ));
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
    }
}
