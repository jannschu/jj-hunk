use std::path::{Path, PathBuf};
use std::process::Command;

/// A temporary jj repo that is fully isolated from any ambient git/jj state.
/// Cleaned up on drop.
struct TestRepo {
    dir: PathBuf,
    config_path: PathBuf,
}

impl TestRepo {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("jj-hunk-test-{}-{}", name, std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();

        // Init a git-backed jj repo
        let out = Command::new("jj")
            .args(["git", "init"])
            .current_dir(&dir)
            .env("JJ_USER", "Test User")
            .env("JJ_EMAIL", "test@example.com")
            .env_remove("JJ_CONFIG")
            .output()
            .expect("jj git init failed");
        assert!(
            out.status.success(),
            "jj git init: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // Write a jj config file for merge-tool setup.
        // This is passed via JJ_CONFIG to all jj invocations (including
        // those spawned by jj-hunk internally).
        let config_path = dir.join("_jj_config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[merge-tools.jj-hunk]\nprogram = {:?}\nedit-args = [\"select\", \"$left\", \"$right\"]\n",
                jj_hunk_bin(),
            ),
        ).unwrap();

        Self { dir, config_path }
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn write_file(&self, name: &str, content: &str) {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn jj(&self, args: &[&str]) -> std::process::Output {
        Command::new("jj")
            .args(args)
            .current_dir(&self.dir)
            .env("JJ_USER", "Test User")
            .env("JJ_EMAIL", "test@example.com")
            .env("JJ_CONFIG", &self.config_path)
            .env("PATH", path_with_jj_hunk())
            .output()
            .expect("failed to run jj")
    }

    fn jj_ok(&self, args: &[&str]) -> String {
        let out = self.jj(args);
        assert!(
            out.status.success(),
            "jj {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn hunk(&self, args: &[&str]) -> std::process::Output {
        Command::new(jj_hunk_bin())
            .args(args)
            .current_dir(&self.dir)
            .env("JJ_USER", "Test User")
            .env("JJ_EMAIL", "test@example.com")
            .env("JJ_CONFIG", &self.config_path)
            .env("PATH", path_with_jj_hunk())
            .output()
            .expect("failed to run jj-hunk")
    }

    fn hunk_with_empty_config(&self, args: &[&str]) -> std::process::Output {
        let config = self.dir.join("empty-jj-config.toml");
        std::fs::write(&config, "").unwrap();

        Command::new(jj_hunk_bin())
            .args(args)
            .current_dir(&self.dir)
            .env("JJ_USER", "Test User")
            .env("JJ_EMAIL", "test@example.com")
            .env("JJ_CONFIG", config)
            .output()
            .expect("failed to run jj-hunk")
    }

    fn hunk_ok(&self, args: &[&str]) -> String {
        let out = self.hunk(args);
        assert!(
            out.status.success(),
            "jj-hunk {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn hunk_ok_with_empty_config(&self, args: &[&str]) -> String {
        let out = self.hunk_with_empty_config(args);
        assert!(
            out.status.success(),
            "jj-hunk {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn hunk_fail(&self, args: &[&str]) -> String {
        let out = self.hunk(args);
        assert!(
            !out.status.success(),
            "jj-hunk {:?} should have failed but succeeded: {}",
            args,
            String::from_utf8_lossy(&out.stdout),
        );
        let mut combined = String::from_utf8_lossy(&out.stderr).to_string();
        combined.push_str(&String::from_utf8_lossy(&out.stdout));
        combined
    }

    /// Get the log as a simple list of descriptions (most recent first).
    fn log_descriptions(&self) -> Vec<String> {
        let out = self.jj_ok(&[
            "log",
            "--no-graph",
            "-T",
            r#"if(description, description.first_line() ++ "\n", "")"#,
        ]);
        out.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Show files changed in a revision.
    fn changed_files(&self, rev: &str) -> Vec<String> {
        let out = self.jj_ok(&["diff", "-r", rev, "--summary"]);
        out.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Build a PATH that includes the directory containing the jj-hunk binary,
/// so that `jj --tool=jj-hunk` can find it.
fn path_with_jj_hunk() -> String {
    let bin_dir = jj_hunk_bin().parent().unwrap().to_path_buf();
    let current_path = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", bin_dir.display(), current_path)
}

fn jj_hunk_bin() -> PathBuf {
    // Use the binary built by `cargo test`
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    path.push("jj-hunk");
    assert!(
        path.exists(),
        "jj-hunk binary not found at {:?}. Run `cargo build` first.",
        path
    );
    path
}

// ---------------------------------------------------------------------------
// list -r
// ---------------------------------------------------------------------------

#[test]
fn list_rev_shows_hunks_for_non_working_copy() {
    let repo = TestRepo::new("list-rev");

    // Create a commit with some content
    repo.write_file("a.txt", "line1\nline2\n");
    repo.jj_ok(&["commit", "-m", "add a.txt"]);

    // Make a second commit that modifies a.txt
    repo.write_file("a.txt", "line1\nLINE2\n");
    repo.jj_ok(&["commit", "-m", "modify a.txt"]);

    // Working copy is now empty — list @ should have nothing
    let list_wc = repo.hunk_ok(&["list"]);
    assert!(
        !list_wc.contains("a.txt"),
        "working copy should have no hunks for a.txt"
    );

    // list -r @- should show the modification
    let list_prev = repo.hunk_ok(&["list", "-r", "@-"]);
    assert!(
        list_prev.contains("a.txt"),
        "list -r @- should show a.txt hunks:\n{}",
        list_prev
    );
    assert!(list_prev.contains("LINE2"));
}

#[test]
fn list_rev_files_mode() {
    let repo = TestRepo::new("list-rev-files");

    repo.write_file("foo.txt", "hello\n");
    repo.write_file("bar.txt", "world\n");
    repo.jj_ok(&["commit", "-m", "initial"]);

    repo.write_file("foo.txt", "hello changed\n");
    repo.write_file("bar.txt", "world changed\n");
    repo.jj_ok(&["commit", "-m", "changes"]);

    let out = repo.hunk_ok(&["list", "-r", "@-", "--files"]);
    assert!(out.contains("foo.txt"));
    assert!(out.contains("bar.txt"));
}

#[test]
fn list_merge_commit_shows_only_changes_from_merged_parent_tree() {
    let repo = TestRepo::new("list-merge");

    repo.write_file("a-file", "1\n2\n3\n");
    repo.write_file("unmodified", "4\n5\n6\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.jj_ok(&["new", "@-", "-m", "A"]);
    repo.write_file("a-file", "AAAA1\n2\n3\n");

    repo.jj_ok(&["new", "@-", "-m", "B"]);
    repo.write_file("a-file", "1\n2\nBBBB1\n");

    repo.jj_ok(&["new", "heads(all())", "-m", "M"]);
    repo.write_file("a-file", "AAAA1\nMMMM2\nBBBB1\n");

    let output = repo.hunk_ok(&["list"]);
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "unexpected list output: {output}");
    assert_eq!(files[0]["path"], "a-file");

    let hunks = files[0]["hunks"].as_array().unwrap();
    assert_eq!(hunks.len(), 1, "unexpected list output: {output}");
    assert_eq!(hunks[0]["removed"], "2\n");
    assert_eq!(hunks[0]["added"], "MMMM2\n");
}

#[test]
fn list_merge_commit_materializes_conflicted_parent_tree() {
    let repo = TestRepo::new("list-merge-conflict");

    repo.write_file("conflicted", "base\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.jj_ok(&["new", "@-", "-m", "A"]);
    repo.write_file("conflicted", "left\n");

    repo.jj_ok(&["new", "@-", "-m", "B"]);
    repo.write_file("conflicted", "right\n");

    repo.jj_ok(&["new", "heads(all())", "-m", "M"]);
    repo.write_file("conflicted", "resolved\n");

    let output = repo.hunk_ok(&["list"]);
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let hunks = value["files"][0]["hunks"].as_array().unwrap();
    assert_eq!(hunks.len(), 1, "unexpected list output: {output}");

    let removed = hunks[0]["removed"].as_str().unwrap();
    assert!(removed.contains("left"), "unexpected list output: {output}");
    assert!(
        removed.contains("right"),
        "unexpected list output: {output}"
    );
    assert_eq!(hunks[0]["added"], "resolved\n");
}

// ---------------------------------------------------------------------------
// split -r
// ---------------------------------------------------------------------------

#[test]
fn split_rev_splits_non_working_copy_revision() {
    let repo = TestRepo::new("split-rev");

    // Base commit
    repo.write_file("a.txt", "aaa\n");
    repo.write_file("b.txt", "bbb\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    // A commit that touches both files
    repo.write_file("a.txt", "AAA\n");
    repo.write_file("b.txt", "BBB\n");
    repo.jj_ok(&["commit", "-m", "modify both"]);

    // Now split @- keeping only a.txt changes in the first commit
    let spec = r#"{"files": {"a.txt": {"action": "keep"}}, "default": "reset"}"#;
    repo.hunk_ok(&["split", "-r", "@-", spec, "only a.txt changes"]);

    // Should now have: base -> "only a.txt changes" -> (rest) -> @
    let log = repo.log_descriptions();
    assert!(
        log.iter().any(|d| d == "only a.txt changes"),
        "should have the split commit: {:?}",
        log
    );

    // The first split commit should only touch a.txt
    // Find the commit by description
    let diff_out = repo.jj_ok(&[
        "log",
        "--no-graph",
        "-r",
        r#"description(substring:"only a.txt changes")"#,
        "-T",
        "change_id ++ \"\n\"",
    ]);
    let change_id = diff_out.trim();
    assert!(!change_id.is_empty(), "should find split commit");

    let files = repo.changed_files(change_id);
    let has_a = files.iter().any(|f| f.contains("a.txt"));
    let has_b = files.iter().any(|f| f.contains("b.txt"));
    assert!(
        has_a,
        "split commit should contain a.txt changes: {:?}",
        files
    );
    assert!(
        !has_b,
        "split commit should NOT contain b.txt changes: {:?}",
        files
    );
}

#[test]
fn split_rev_with_spec_file() {
    let repo = TestRepo::new("split-rev-specfile");

    repo.write_file("x.txt", "xxx\n");
    repo.write_file("y.txt", "yyy\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.write_file("x.txt", "XXX\n");
    repo.write_file("y.txt", "YYY\n");
    repo.jj_ok(&["commit", "-m", "modify both"]);

    // Write spec to a file
    let spec_path = repo.path().join("_spec.json");
    std::fs::write(
        &spec_path,
        r#"{"files": {"x.txt": {"action": "keep"}}, "default": "reset"}"#,
    )
    .unwrap();

    repo.hunk_ok(&[
        "split",
        "-r",
        "@-",
        "-f",
        spec_path.to_str().unwrap(),
        "x only",
    ]);

    let log = repo.log_descriptions();
    assert!(log.iter().any(|d| d == "x only"), "log: {:?}", log);
}

#[test]
fn split_without_rev_operates_on_working_copy() {
    let repo = TestRepo::new("split-no-rev");

    repo.write_file("a.txt", "aaa\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    // Changes in working copy
    repo.write_file("a.txt", "AAA\n");
    repo.write_file("b.txt", "BBB\n");

    let spec = r#"{"files": {"a.txt": {"action": "keep"}}, "default": "reset"}"#;
    repo.hunk_ok(&["split", spec, "a changes"]);

    let log = repo.log_descriptions();
    assert!(
        log.iter().any(|d| d == "a changes"),
        "should have the split commit: {:?}",
        log
    );
}

// ---------------------------------------------------------------------------
// squash -r
// ---------------------------------------------------------------------------

#[test]
fn squash_rev_squashes_non_working_copy_into_parent() {
    let repo = TestRepo::new("squash-rev");

    // Base with a.txt
    repo.write_file("a.txt", "aaa\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    // Commit that adds b.txt and c.txt
    repo.write_file("b.txt", "bbb\n");
    repo.write_file("c.txt", "ccc\n");
    repo.jj_ok(&["commit", "-m", "add b and c"]);

    // Empty working copy now. Squash only b.txt from @- into its parent.
    let spec = r#"{"files": {"b.txt": {"action": "keep"}}, "default": "reset"}"#;
    repo.hunk_ok(&["squash", "-r", "@-", spec]);

    // The parent ("base") should now contain b.txt
    let base_files = repo.changed_files(r#"description(substring:"base")"#);
    let has_b = base_files.iter().any(|f| f.contains("b.txt"));
    assert!(has_b, "base should now have b.txt: {:?}", base_files);

    // @- should still have c.txt but not b.txt
    let mid_files = repo.changed_files("@-");
    let has_c = mid_files.iter().any(|f| f.contains("c.txt"));
    let still_has_b = mid_files.iter().any(|f| f.contains("b.txt"));
    assert!(has_c, "@- should still have c.txt: {:?}", mid_files);
    assert!(
        !still_has_b,
        "@- should NOT have b.txt anymore: {:?}",
        mid_files
    );
}

#[test]
fn squash_without_rev_operates_on_working_copy() {
    let repo = TestRepo::new("squash-no-rev");

    repo.write_file("a.txt", "aaa\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    // Working copy changes
    repo.write_file("a.txt", "AAA\n");
    repo.write_file("b.txt", "BBB\n");

    let spec = r#"{"files": {"a.txt": {"action": "keep"}}, "default": "reset"}"#;
    repo.hunk_ok(&["squash", spec]);

    // a.txt change should be squashed into base
    let base_files = repo.changed_files(r#"description(substring:"base")"#);
    let has_a = base_files.iter().any(|f| f.contains("a.txt"));
    assert!(has_a, "base should have a.txt: {:?}", base_files);
}

// ---------------------------------------------------------------------------
// commit (no -r, sanity check)
// ---------------------------------------------------------------------------

#[test]
fn commit_works_on_working_copy() {
    let repo = TestRepo::new("commit-wc");

    repo.write_file("a.txt", "aaa\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.write_file("a.txt", "AAA\n");
    repo.write_file("b.txt", "BBB\n");

    let spec = r#"{"files": {"a.txt": {"action": "keep"}}, "default": "reset"}"#;
    repo.hunk_ok(&["commit", spec, "commit a only"]);

    let log = repo.log_descriptions();
    assert!(log.iter().any(|d| d == "commit a only"), "log: {:?}", log);

    // b.txt should still be in working copy
    let wc_files = repo.changed_files("@");
    let has_b = wc_files.iter().any(|f| f.contains("b.txt"));
    assert!(has_b, "b.txt should remain in working copy: {:?}", wc_files);
}

#[test]
fn commit_self_configures_jj_hunk_tool_when_user_config_is_empty() {
    let repo = TestRepo::new("commit-self-configures");

    repo.write_file("a.txt", "aaa\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.write_file("a.txt", "AAA\n");
    repo.write_file("b.txt", "BBB\n");

    let spec = r#"{"files": {"a.txt": {"action": "keep"}}, "default": "reset"}"#;
    repo.hunk_ok_with_empty_config(&["commit", spec, "commit a via self config"]);

    let log = repo.log_descriptions();
    assert!(
        log.iter().any(|d| d == "commit a via self config"),
        "log: {:?}",
        log
    );
}

#[test]
fn query_empty_selection_is_a_no_op_for_all_mutation_commands() {
    let repo = TestRepo::new("query-empty-mutations");
    repo.write_file("base.txt", "old\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("base.txt", "new\n");
    repo.write_file("created.txt", "created\n");

    let revision_before = repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]);
    let diff_before = repo.jj_ok(&["diff", "--git"]);
    repo.hunk_ok(&["commit", "--query", "none()", "empty commit"]);
    assert_eq!(
        repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]),
        revision_before
    );
    assert_eq!(repo.jj_ok(&["diff", "--git"]), diff_before);

    repo.hunk_ok(&["split", "--query", "none()", "empty split"]);
    assert_eq!(
        repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]),
        revision_before
    );
    assert_eq!(repo.jj_ok(&["diff", "--git"]), diff_before);

    repo.hunk_ok(&["squash", "--query", "none()"]);
    assert_eq!(
        repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]),
        revision_before
    );
    assert_eq!(repo.jj_ok(&["diff", "--git"]), diff_before);
}

#[test]
fn mutation_queries_reject_ambiguous_selection_inputs() {
    let repo = TestRepo::new("query-ambiguous-mutations");
    repo.write_file("changed.txt", "changed\n");

    for args in [
        ["split", "--query", "all()", "spec", "message"].as_slice(),
        ["commit", "--query", "all()", "spec", "message"].as_slice(),
        ["squash", "--query", "all()", "spec"].as_slice(),
    ] {
        let error = repo.hunk_fail(args);
        assert!(
            error.contains("selection spec"),
            "unexpected error for {args:?}: {error}"
        );
    }
}

#[test]
fn empty_text_list_output_is_valid() {
    let repo = TestRepo::new("empty-text-list");
    repo.write_file("base.txt", "base\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    assert_eq!(repo.hunk_ok(&["list", "--format", "text"]), "");
    repo.write_file("base.txt", "changed\n");
    assert_eq!(
        repo.hunk_ok(&["list", "--format", "text", "--query", "none()"]),
        ""
    );
}

#[cfg(unix)]
#[test]
fn mutation_query_rejects_unsupported_input_before_writes() {
    let repo = TestRepo::new("query-unsupported-mutation");
    repo.write_file("base.txt", "old\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("base.txt", "new\n");
    std::os::unix::fs::symlink("base.txt", repo.path().join("link.txt")).unwrap();

    let revision_before = repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]);
    let diff_before = repo.jj_ok(&["diff", "--git"]);
    let error = repo.hunk_fail(&["commit", "--query", "all()", "must fail"]);

    assert!(error.contains("special-file changes"), "{error}");
    assert_eq!(
        repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]),
        revision_before
    );
    assert_eq!(repo.jj_ok(&["diff", "--git"]), diff_before);
}

#[cfg(unix)]
#[test]
fn binary_query_keeps_unselected_mode_change() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::new("query-binary-with-mode");
    repo.write_file("tool.bin", "old\0bytes");
    let path = repo.path().join("tool.bin");
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.write_file("tool.bin", "new\0bytes");
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o644);
    std::fs::set_permissions(&path, permissions).unwrap();
    repo.hunk_ok(&["commit", "--query", "binaries()", "binary only"]);

    assert_eq!(
        repo.jj(&["file", "show", "-r", "@-", "tool.bin"]).stdout,
        b"new\0bytes"
    );
    assert!(!repo
        .jj_ok(&["diff", "--git", "-r", "@-"])
        .contains("mode 100644"));
    let remaining = repo.jj_ok(&["diff", "--git", "-r", "@"]);
    assert!(remaining.contains("old mode 100755"), "{remaining}");
    assert!(remaining.contains("new mode 100644"), "{remaining}");
}

#[test]
fn creation_query_rejects_retained_directory_collision_without_writes() {
    let repo = TestRepo::new("query-retained-directory-collision");
    repo.write_file("item/retained.txt", "keep\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::remove_file(repo.path().join("item/retained.txt")).unwrap();
    std::fs::remove_dir(repo.path().join("item")).unwrap();
    repo.write_file("item", "replacement\n");

    let revision_before = repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]);
    let diff_before = repo.jj_ok(&["diff", "--git"]);
    let error = repo.hunk_fail(&["commit", "--query", "creations()", "must fail"]);

    assert!(error.contains("collides with a retained path"), "{error}");
    assert_eq!(
        repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]),
        revision_before
    );
    assert_eq!(repo.jj_ok(&["diff", "--git"]), diff_before);
}

#[test]
fn commit_query_selects_creation_and_leaves_other_changes() {
    let repo = TestRepo::new("commit-query");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("selected.txt", "selected\n");
    repo.write_file("remaining.txt", "remaining\n");

    repo.hunk_ok(&[
        "commit",
        "--query",
        "files(\"selected.txt\") & creations()",
        "selected creation",
    ]);

    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "selected.txt"]),
        "selected\n"
    );
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "remaining.txt"])
        .status
        .success());
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@", "remaining.txt"]),
        "remaining\n"
    );
    assert_eq!(repo.changed_files("@"), vec!["A remaining.txt"]);
}

#[test]
fn occurrence_id_roundtrips_through_all_query_mutations() {
    for command in ["commit", "split", "squash"] {
        let repo = TestRepo::new(&format!("occurrence-id-{command}"));
        repo.write_file("selected.txt", "old\n");
        repo.write_file("remaining.txt", "old\n");
        repo.jj_ok(&["commit", "-m", "base"]);
        repo.write_file("selected.txt", "new\n");
        repo.write_file("remaining.txt", "new\n");

        let preview = repo.hunk_ok(&["list", "--query", "files(\"selected.txt\")"]);
        let value: serde_json::Value = serde_json::from_str(&preview).unwrap();
        let id = value["files"][0]["hunks"][0]["id"].as_str().unwrap();
        let query = format!("id(\"{id}\")");
        if command == "squash" {
            repo.hunk_ok(&[command, "--query", &query]);
        } else {
            repo.hunk_ok(&[command, "--query", &query, "selected by id"]);
        }

        assert_eq!(
            repo.jj_ok(&["file", "show", "-r", "@-", "selected.txt"]),
            "new\n"
        );
        assert_eq!(
            repo.jj_ok(&["file", "show", "-r", "@-", "remaining.txt"]),
            "old\n"
        );
        assert_eq!(
            repo.jj_ok(&["file", "show", "-r", "@", "remaining.txt"]),
            "new\n"
        );
    }
}

#[test]
fn normal_creation_id_matches_query_file_unit_for_all_mutations() {
    for command in ["commit", "split", "squash"] {
        let repo = TestRepo::new(&format!("occurrence-id-creation-{command}"));
        repo.jj_ok(&["commit", "-m", "base"]);
        repo.write_file("created.txt", "created\n");
        repo.write_file("remaining.txt", "remaining\n");

        let ordinary: serde_json::Value = serde_json::from_str(&repo.hunk_ok(&["list"])).unwrap();
        let ordinary_id = ordinary["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "created.txt")
            .unwrap()["hunks"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let query_output: serde_json::Value =
            serde_json::from_str(&repo.hunk_ok(&["list", "--query", "files(\"created.txt\")"]))
                .unwrap();
        assert_eq!(query_output["files"][0]["file_units"][0]["id"], ordinary_id);

        let query = format!("id(\"{ordinary_id}\")");
        if command == "squash" {
            repo.hunk_ok(&[command, "--query", &query]);
        } else {
            repo.hunk_ok(&[command, "--query", &query, "creation by id"]);
        }
        assert_eq!(
            repo.jj_ok(&["file", "show", "-r", "@-", "created.txt"]),
            "created\n"
        );
        assert!(!repo
            .jj(&["file", "show", "-r", "@-", "remaining.txt"])
            .status
            .success());
    }
}

#[test]
fn listed_text_id_roundtrips_through_the_legacy_spec() {
    let repo = TestRepo::new("occurrence-id-spec");
    repo.write_file("selected.txt", "old\n");
    repo.write_file("remaining.txt", "old\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("selected.txt", "new\n");
    repo.write_file("remaining.txt", "new\n");

    let output: serde_json::Value = serde_json::from_str(&repo.hunk_ok(&["list"])).unwrap();
    let selected = output["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "selected.txt")
        .unwrap();
    let id = selected["hunks"][0]["id"].as_str().unwrap();
    let spec = format!(r#"{{"files":{{"selected.txt":{{"ids":["{id}"]}}}},"default":"reset"}}"#);
    repo.hunk_ok(&["commit", &spec, "selected by spec id"]);

    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "selected.txt"]),
        "new\n"
    );
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "remaining.txt"]),
        "old\n"
    );
}

#[test]
fn listed_creation_id_roundtrips_through_the_legacy_spec() {
    let repo = TestRepo::new("occurrence-id-creation-spec");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("created.txt", "created\n");
    repo.write_file("remaining.txt", "remaining\n");

    let output: serde_json::Value = serde_json::from_str(&repo.hunk_ok(&["list"])).unwrap();
    let created = output["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "created.txt")
        .unwrap();
    let id = created["hunks"][0]["id"].as_str().unwrap();
    let spec = format!(r#"{{"files":{{"created.txt":{{"ids":["{id}"]}}}},"default":"reset"}}"#);
    repo.hunk_ok(&["commit", &spec, "creation by spec id"]);

    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "created.txt"]),
        "created\n"
    );
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "remaining.txt"])
        .status
        .success());
}

fn different_valid_id(id: &str) -> String {
    let mut changed = id.to_owned();
    let replacement = if changed.ends_with('0') { '1' } else { '0' };
    changed.pop();
    changed.push(replacement);
    changed
}

#[test]
fn stale_creation_and_deletion_ids_select_no_file_operation() {
    let creation = TestRepo::new("occurrence-id-stale-creation");
    creation.jj_ok(&["commit", "-m", "base"]);
    creation.write_file("item.txt", "created\n");
    let output: serde_json::Value = serde_json::from_str(&creation.hunk_ok(&["list"])).unwrap();
    let stale = different_valid_id(output["files"][0]["hunks"][0]["id"].as_str().unwrap());
    let spec = format!(r#"{{"files":{{"item.txt":{{"ids":["{stale}"]}}}}}}"#);
    creation.hunk_ok(&["commit", &spec, "stale creation"]);
    assert!(!creation
        .jj(&["file", "show", "-r", "@-", "item.txt"])
        .status
        .success());

    let deletion = TestRepo::new("occurrence-id-stale-deletion");
    deletion.write_file("item.txt", "deleted\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            deletion.path().join("item.txt"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    deletion.jj_ok(&["commit", "-m", "base"]);
    std::fs::remove_file(deletion.path().join("item.txt")).unwrap();
    let output: serde_json::Value = serde_json::from_str(&deletion.hunk_ok(&["list"])).unwrap();
    let stale = different_valid_id(output["files"][0]["hunks"][0]["id"].as_str().unwrap());
    let spec = format!(r#"{{"files":{{"item.txt":{{"ids":["{stale}"]}}}}}}"#);
    deletion.hunk_ok(&["commit", &spec, "stale deletion"]);
    assert_eq!(
        deletion.jj_ok(&["file", "show", "-r", "@-", "item.txt"]),
        "deleted\n"
    );
    assert!(deletion.changed_files("@-").is_empty());
}

#[test]
fn empty_file_unit_ids_are_not_legacy_hunk_selectors() {
    let creation = TestRepo::new("occurrence-id-empty-creation-spec");
    creation.jj_ok(&["commit", "-m", "base"]);
    creation.write_file("empty.txt", "");
    let output: serde_json::Value =
        serde_json::from_str(&creation.hunk_ok(&["list", "--query", "files(\"empty.txt\")"]))
            .unwrap();
    let id = output["files"][0]["file_units"][0]["id"].as_str().unwrap();
    let spec = format!(r#"{{"files":{{"empty.txt":{{"ids":["{id}"]}}}}}}"#);
    creation.hunk_ok(&["commit", &spec, "empty creation ID ignored"]);
    assert!(!creation
        .jj(&["file", "show", "-r", "@-", "empty.txt"])
        .status
        .success());

    let deletion = TestRepo::new("occurrence-id-empty-deletion-spec");
    deletion.write_file("empty.txt", "");
    deletion.jj_ok(&["commit", "-m", "base"]);
    std::fs::remove_file(deletion.path().join("empty.txt")).unwrap();
    let output: serde_json::Value =
        serde_json::from_str(&deletion.hunk_ok(&["list", "--query", "files(\"empty.txt\")"]))
            .unwrap();
    let id = output["files"][0]["file_units"][0]["id"].as_str().unwrap();
    let spec = format!(r#"{{"files":{{"empty.txt":{{"ids":["{id}"]}}}}}}"#);
    deletion.hunk_ok(&["commit", &spec, "empty deletion ID ignored"]);
    assert_eq!(
        deletion.jj_ok(&["file", "show", "-r", "@-", "empty.txt"]),
        ""
    );
}

#[test]
fn listed_deletion_id_keeps_the_deletion_in_a_legacy_spec() {
    let repo = TestRepo::new("occurrence-id-deletion-spec");
    repo.write_file("item.txt", "deleted\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::remove_file(repo.path().join("item.txt")).unwrap();
    let output: serde_json::Value = serde_json::from_str(&repo.hunk_ok(&["list"])).unwrap();
    let id = output["files"][0]["hunks"][0]["id"].as_str().unwrap();
    let spec = format!(r#"{{"files":{{"item.txt":{{"ids":["{id}"]}}}}}}"#);
    repo.hunk_ok(&["commit", &spec, "deletion by id"]);
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "item.txt"])
        .status
        .success());
}

#[test]
fn legacy_hunk_selection_rejects_renames_before_mutation() {
    let repo = TestRepo::new("occurrence-id-rename-spec-rejected");
    prepare_edited_rename(&repo);
    let before = repo.jj_ok(&["diff", "--git"]);
    let id = format!("hunk-{}", "1".repeat(64));
    let spec =
        format!(r#"{{"files":{{"archive/client.rs":{{"ids":["{id}"]}}}},"default":"reset"}}"#);
    let error = repo.hunk_fail(&["commit", &spec, "must fail"]);
    assert!(error.contains("does not support renamed files"), "{error}");
    assert_eq!(repo.jj_ok(&["diff", "--git"]), before);
}

#[test]
fn occurrence_ids_are_stable_across_formats_and_display_limits() {
    let repo = TestRepo::new("occurrence-id-stability");
    repo.write_file("item.txt", "old first\ncontext\nold last\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("item.txt", "new first\ncontext\nnew last\n");

    let full: serde_json::Value = serde_json::from_str(&repo.hunk_ok(&["list"])).unwrap();
    let limited: serde_json::Value =
        serde_json::from_str(&repo.hunk_ok(&["list", "--max-bytes", "1", "--max-lines", "1"]))
            .unwrap();
    assert_eq!(
        full["files"][0]["hunks"][0]["id"],
        limited["files"][0]["hunks"][0]["id"]
    );

    let yaml = repo.hunk_ok(&["list", "--query", "all()", "--format", "yaml"]);
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(
        full["files"][0]["hunks"][0]["id"].as_str(),
        yaml_value["files"][0]["hunks"][0]["id"].as_str()
    );
}

#[test]
fn occurrence_id_changes_when_the_full_comparison_changes() {
    let repo = TestRepo::new("occurrence-id-comparison");
    repo.write_file("selected.txt", "old\n");
    repo.write_file("other.txt", "old\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("selected.txt", "new\n");
    let first: serde_json::Value =
        serde_json::from_str(&repo.hunk_ok(&["list", "--query", "files(\"selected.txt\")"]))
            .unwrap();
    repo.write_file("other.txt", "changed\n");
    let second: serde_json::Value =
        serde_json::from_str(&repo.hunk_ok(&["list", "--query", "files(\"selected.txt\")"]))
            .unwrap();
    assert_ne!(
        first["files"][0]["hunks"][0]["id"],
        second["files"][0]["hunks"][0]["id"]
    );
}

fn prepare_edited_rename(repo: &TestRepo) {
    repo.write_file("src/client.rs", "header\ntimeout = 10\nfooter\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::create_dir_all(repo.path().join("archive")).unwrap();
    std::fs::rename(
        repo.path().join("src/client.rs"),
        repo.path().join("archive/client.rs"),
    )
    .unwrap();
    repo.write_file("archive/client.rs", "header\ntimeout = 20\nfooter\n");
}

#[test]
fn split_query_rename_only_keeps_old_content() {
    let repo = TestRepo::new("split-query-rename-only");
    prepare_edited_rename(&repo);

    repo.hunk_ok(&["split", "--query", "renames()", "rename only"]);
    let revision = repo
        .jj_ok(&[
            "log",
            "--no-graph",
            "-r",
            "description(substring:\"rename only\")",
            "-T",
            "change_id",
        ])
        .trim()
        .to_owned();
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", &revision, "archive/client.rs"]),
        "header\ntimeout = 10\nfooter\n"
    );
    assert!(!repo
        .jj(&["file", "show", "-r", &revision, "src/client.rs"])
        .status
        .success());
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@", "archive/client.rs"]),
        "header\ntimeout = 20\nfooter\n"
    );
}

#[test]
fn commit_query_text_only_keeps_old_path() {
    let repo = TestRepo::new("commit-query-text-only");
    prepare_edited_rename(&repo);

    repo.hunk_ok(&["commit", "--query", "content(\"timeout\")", "text only"]);
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "src/client.rs"]),
        "header\ntimeout = 20\nfooter\n"
    );
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "archive/client.rs"])
        .status
        .success());
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@", "archive/client.rs"]),
        "header\ntimeout = 20\nfooter\n"
    );
}

#[test]
fn side_regex_preview_and_commit_select_the_same_text_occurrence() {
    let repo = TestRepo::new("query-side-regex-preview-commit");
    prepare_edited_rename(&repo);
    let query = r#"after_files("archive/**") & added_regex("timeout\\s*=\\s*20")"#;

    let preview = repo.hunk_ok(&["list", "--query", query]);
    let value: serde_json::Value = serde_json::from_str(&preview).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "archive/client.rs");
    assert_eq!(files[0]["hunks"].as_array().unwrap().len(), 1);
    assert!(files[0]["file_units"].is_null());

    repo.hunk_ok(&["commit", "--query", query, "side regex"]);
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "src/client.rs"]),
        "header\ntimeout = 20\nfooter\n"
    );
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "archive/client.rs"])
        .status
        .success());
}

#[test]
fn invalid_regex_fails_before_mutation() {
    let repo = TestRepo::new("query-invalid-regex-mutation");
    repo.write_file("base.txt", "old\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("base.txt", "new\n");

    let revision_before = repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]);
    let diff_before = repo.jj_ok(&["diff", "--git"]);
    let error = repo.hunk_fail(&["commit", "--query", "regex(\"[\")", "must fail"]);

    assert!(error.contains("invalid regex"), "{error}");
    assert!(error.contains("byte 6"), "{error}");
    assert_eq!(
        repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]),
        revision_before
    );
    assert_eq!(repo.jj_ok(&["diff", "--git"]), diff_before);
}

#[test]
fn alias_preview_and_commit_match_handwritten_expansion() {
    let repo = TestRepo::new("query-alias-preview-commit");
    repo.write_file("src/hand.rs", "timeout = 10\n");
    repo.write_file("generated/code.rs", "timeout = 10\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("src/hand.rs", "timeout = 20\n");
    repo.write_file("generated/code.rs", "timeout = 20\n");

    let generated = "generated()=files(\"generated/**\")";
    let handwritten = "handwritten(selection)=(all() ~ generated()) & selection()";
    let query = "handwritten(content(\"timeout\"))";
    let expanded = "(all() ~ files(\"generated/**\")) & content(\"timeout\")";
    let aliased_preview = repo.hunk_ok(&[
        "--alias",
        generated,
        "--alias",
        handwritten,
        "list",
        "--query",
        query,
    ]);
    let expanded_preview = repo.hunk_ok(&["list", "--query", expanded]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&aliased_preview).unwrap(),
        serde_json::from_str::<serde_json::Value>(&expanded_preview).unwrap()
    );

    repo.hunk_ok(&[
        "commit",
        "--alias",
        generated,
        "--alias",
        handwritten,
        "--query",
        query,
        "handwritten timeout",
    ]);
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "src/hand.rs"]),
        "timeout = 20\n"
    );
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "generated/code.rs"]),
        "timeout = 10\n"
    );
    assert_eq!(repo.changed_files("@"), vec!["M generated/code.rs"]);
}

#[test]
fn invalid_alias_configuration_fails_before_mutation() {
    let repo = TestRepo::new("query-invalid-alias-mutation");
    repo.write_file("base.txt", "old\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("base.txt", "new\n");

    let revision_before = repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]);
    let diff_before = repo.jj_ok(&["diff", "--git"]);
    let error = repo.hunk_fail(&[
        "commit",
        "--alias",
        "content()=all()",
        "--query",
        "all()",
        "must fail",
    ]);

    assert!(error.contains("collides with a builtin"), "{error}");
    assert_eq!(
        repo.jj_ok(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]),
        revision_before
    );
    assert_eq!(repo.jj_ok(&["diff", "--git"]), diff_before);
}

#[test]
fn squash_query_combined_rename_and_text_applies_both() {
    let repo = TestRepo::new("squash-query-combined");
    prepare_edited_rename(&repo);

    repo.hunk_ok(&["squash", "--query", "renames() | content(\"timeout\")"]);
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "archive/client.rs"]),
        "header\ntimeout = 20\nfooter\n"
    );
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "src/client.rs"])
        .status
        .success());
    assert!(repo.changed_files("@").is_empty());
}

#[cfg(unix)]
#[test]
fn commit_query_applies_file_units_and_preserves_unselected_text() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::new("commit-query-file-units");
    repo.write_file("deleted.txt", "deleted\n");
    repo.write_file("empty-deleted.txt", "");
    repo.write_file("binary.bin", "old\0bytes");
    repo.write_file("mode.sh", "unchanged\n");
    repo.write_file("remaining.txt", "old\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    std::fs::remove_file(repo.path().join("deleted.txt")).unwrap();
    std::fs::remove_file(repo.path().join("empty-deleted.txt")).unwrap();
    repo.write_file("created.txt", "created\n");
    repo.write_file("empty-created.txt", "");
    repo.write_file("binary.bin", "new\0bytes");
    repo.write_file("remaining.txt", "new\n");
    let mode_path = repo.path().join("mode.sh");
    let mut permissions = std::fs::metadata(&mode_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(mode_path, permissions).unwrap();

    repo.hunk_ok(&[
        "commit",
        "--query",
        "creations() | deletions() | binaries() | modes()",
        "file units",
    ]);

    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "created.txt"]),
        "created\n"
    );
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "empty-created.txt"]),
        ""
    );
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "deleted.txt"])
        .status
        .success());
    assert!(!repo
        .jj(&["file", "show", "-r", "@-", "empty-deleted.txt"])
        .status
        .success());
    assert_eq!(
        repo.jj(&["file", "show", "-r", "@-", "binary.bin"]).stdout,
        b"new\0bytes"
    );
    assert_eq!(
        repo.jj_ok(&["file", "show", "-r", "@-", "remaining.txt"]),
        "old\n"
    );
    assert!(repo
        .jj_ok(&["diff", "--git", "-r", "@-"])
        .contains("new mode 100755"));
    assert_eq!(repo.changed_files("@"), vec!["M remaining.txt"]);
}

// ---------------------------------------------------------------------------
// error cases
// ---------------------------------------------------------------------------

#[test]
fn split_rev_invalid_revset_fails() {
    let repo = TestRepo::new("split-bad-rev");

    repo.write_file("a.txt", "aaa\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    let spec = r#"{"files": {"a.txt": {"action": "keep"}}, "default": "reset"}"#;
    let err = repo.hunk_fail(&["split", "-r", "nonexistent_bookmark", spec, "msg"]);
    assert!(
        err.contains("failed")
            || err.contains("error")
            || err.contains("Error")
            || err.contains("Revision"),
        "should fail with bad revset: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// list -r with spec preview
// ---------------------------------------------------------------------------

#[test]
fn list_rev_with_spec_filters_output() {
    let repo = TestRepo::new("list-rev-spec");

    repo.write_file("keep.txt", "keep\n");
    repo.write_file("drop.txt", "drop\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.write_file("keep.txt", "KEEP\n");
    repo.write_file("drop.txt", "DROP\n");
    repo.jj_ok(&["commit", "-m", "changes"]);

    // List with a spec that only shows keep.txt
    let spec = r#"{"files": {"keep.txt": {"action": "keep"}}, "default": "reset"}"#;
    let out = repo.hunk_ok(&["list", "-r", "@-", "--spec", spec]);
    assert!(out.contains("keep.txt"), "should show keep.txt:\n{}", out);
    assert!(
        !out.contains("drop.txt"),
        "should NOT show drop.txt:\n{}",
        out
    );
}

#[test]
fn list_query_selects_whole_blocks_in_the_requested_revision() {
    let repo = TestRepo::new("list-query");
    repo.write_file(
        "src/server.rs",
        "header\ntimeout = 10\nmiddle\nretry = false\nfooter\n",
    );
    repo.write_file("other.rs", "timeout = 10\n");
    repo.jj_ok(&["commit", "-m", "base"]);

    repo.write_file(
        "src/server.rs",
        "header\ntimeout = 20\nmiddle\nretry = true\nfooter\n",
    );
    repo.write_file("other.rs", "timeout = 20\n");
    repo.jj_ok(&["commit", "-m", "changes"]);

    let diff_before = repo.jj_ok(&["diff", "--git", "-r", "@-"]);
    let output = repo.hunk_ok(&[
        "list",
        "-r",
        "@-",
        "--query",
        "files(\"src/**\") & content(\"timeout\")",
    ]);
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["files"].as_array().unwrap().len(), 1);
    assert_eq!(value["files"][0]["path"], "src/server.rs");
    let hunks = value["files"][0]["hunks"].as_array().unwrap();
    assert_eq!(hunks.len(), 1, "unexpected query output: {output}");
    assert_eq!(hunks[0]["added"], "timeout = 20\n");
    assert_eq!(repo.jj_ok(&["diff", "--git", "-r", "@-"]), diff_before);
}

#[test]
fn list_query_evaluates_complete_content_before_display_limits() {
    let repo = TestRepo::new("list-query-complete");
    repo.write_file("src/server.rs", "one\ntwo\nthree\ntimeout = 10\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("src/server.rs", "one\ntwo\nthree\ntimeout = 20\n");
    repo.jj_ok(&["commit", "-m", "changes"]);

    let output = repo.hunk_ok(&[
        "list",
        "-r",
        "@-",
        "--query",
        "content(\"timeout\")",
        "--max-lines",
        "1",
    ]);
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["files"][0]["hunks"].as_array().unwrap().len(), 1);
}

#[test]
fn list_query_selects_creation_and_binary_units_and_rejects_conflicting_filters() {
    let repo = TestRepo::new("list-query-errors");
    repo.write_file("created.rs", "timeout\n");

    let creation = repo.hunk_ok(&["list", "--query", "content(\"timeout\")"]);
    let value: serde_json::Value = serde_json::from_str(&creation).unwrap();
    assert_eq!(value["files"][0]["file_units"][0]["kind"], "creation");
    assert_eq!(value["files"][0]["file_units"][0]["added"], "timeout\n");

    let binary_repo = TestRepo::new("list-query-binary");
    binary_repo.jj_ok(&["commit", "-m", "base"]);
    binary_repo.write_file("binary.dat", "value\0binary");
    let binary = binary_repo.hunk_ok(&["list", "--query", "binaries()", "--binary", "skip"]);
    let value: serde_json::Value = serde_json::from_str(&binary).unwrap();
    assert_eq!(value["files"][0]["file_units"][0]["kind"], "binary");

    let include = repo.hunk_fail(&["list", "--query", "all()", "--include", "src/**"]);
    assert!(include.contains("cannot be used with"));

    let spec = repo.hunk_fail(&["list", "--query", "all()", "--spec", "{}"]);
    assert!(spec.contains("cannot be used with"));

    let invalid = repo.hunk_fail(&["list", "--query", "unknown()"]);
    assert!(invalid.contains("Invalid hunkset query"));
    assert!(invalid.contains("unknown function or alias `unknown`"));
}

#[cfg(unix)]
#[test]
fn list_query_keeps_mode_and_text_units_independent() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::new("list-query-mode");
    repo.write_file("script.sh", "old\n");
    repo.write_file("pure-mode.sh", "unchanged\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    repo.write_file("script.sh", "new\n");
    let path = repo.path().join("script.sh");
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
    let pure_mode_path = repo.path().join("pure-mode.sh");
    let mut permissions = std::fs::metadata(&pure_mode_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(pure_mode_path, permissions).unwrap();

    let text = repo.hunk_ok(&["list", "--query", "content(\"new\")"]);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["files"][0]["hunks"].as_array().unwrap().len(), 1);
    assert!(value["files"][0].get("file_units").is_none());

    let mode = repo.hunk_ok(&["list", "--query", "modes()"]);
    let value: serde_json::Value = serde_json::from_str(&mode).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{mode}");
    assert!(files
        .iter()
        .all(|file| file["file_units"][0]["kind"] == "mode"));
    assert!(files
        .iter()
        .all(|file| file["hunks"].as_array().unwrap().is_empty()));
}

#[test]
fn list_query_selects_text_file_units_and_empty_files() {
    let repo = TestRepo::new("list-query-file-units");
    repo.write_file("deleted.txt", "removed content\n");
    repo.write_file("empty-deleted.txt", "");
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::remove_file(repo.path().join("deleted.txt")).unwrap();
    std::fs::remove_file(repo.path().join("empty-deleted.txt")).unwrap();
    repo.write_file("created.txt", "added content\n");
    repo.write_file("empty-created.txt", "");

    let creations = repo.hunk_ok(&["list", "--query", "creations()"]);
    let value: serde_json::Value = serde_json::from_str(&creations).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{creations}");
    assert!(files
        .iter()
        .all(|file| file["file_units"][0]["kind"] == "creation"));
    assert!(files
        .iter()
        .any(|file| file["file_units"][0]["added"] == ""));

    let deletions = repo.hunk_ok(&["list", "--query", "deletions()"]);
    let value: serde_json::Value = serde_json::from_str(&deletions).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{deletions}");
    assert!(files
        .iter()
        .all(|file| file["file_units"][0]["kind"] == "deletion"));
    assert!(files
        .iter()
        .any(|file| file["file_units"][0]["removed"] == ""));

    let content = repo.hunk_ok(&["list", "--query", "content(\"added content\")"]);
    let value: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(value["files"].as_array().unwrap().len(), 1);
    assert_eq!(value["files"][0]["file_units"][0]["kind"], "creation");
}

#[cfg(unix)]
#[test]
fn list_query_keeps_executable_create_and_delete_indivisible() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::new("list-query-executable-create-delete");
    repo.write_file("deleted.sh", "removed\n");
    let deleted_path = repo.path().join("deleted.sh");
    let mut permissions = std::fs::metadata(&deleted_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&deleted_path, permissions).unwrap();
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::remove_file(deleted_path).unwrap();

    repo.write_file("created.sh", "added\n");
    let created_path = repo.path().join("created.sh");
    let mut permissions = std::fs::metadata(&created_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(created_path, permissions).unwrap();

    let output = repo.hunk_ok(&["list", "--query", "all()"]);
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{output}");
    assert!(files
        .iter()
        .all(|file| file["file_units"].as_array().unwrap().len() == 1));
    assert!(files
        .iter()
        .any(|file| file["file_units"][0]["kind"] == "creation"));
    assert!(files
        .iter()
        .any(|file| file["file_units"][0]["kind"] == "deletion"));
}

#[cfg(unix)]
#[test]
fn list_query_does_not_invent_binary_units_for_rename_or_mode_only() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::new("list-query-binary-metadata-only");
    repo.write_file("rename.bin", "same\0bytes");
    repo.write_file("mode.bin", "same\0bytes");
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::rename(
        repo.path().join("rename.bin"),
        repo.path().join("renamed.bin"),
    )
    .unwrap();
    let mode_path = repo.path().join("mode.bin");
    let mut permissions = std::fs::metadata(&mode_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(mode_path, permissions).unwrap();

    let binaries = repo.hunk_ok(&["list", "--query", "binaries()"]);
    let value: serde_json::Value = serde_json::from_str(&binaries).unwrap();
    assert!(value["files"].as_array().unwrap().is_empty(), "{binaries}");

    let renames = repo.hunk_ok(&["list", "--query", "renames()"]);
    let value: serde_json::Value = serde_json::from_str(&renames).unwrap();
    assert_eq!(value["files"].as_array().unwrap().len(), 1, "{renames}");
    assert_eq!(value["files"][0]["file_units"][0]["kind"], "rename");

    let modes = repo.hunk_ok(&["list", "--query", "modes()"]);
    let value: serde_json::Value = serde_json::from_str(&modes).unwrap();
    assert_eq!(value["files"].as_array().unwrap().len(), 1, "{modes}");
    assert_eq!(value["files"][0]["file_units"][0]["kind"], "mode");
}

#[test]
fn list_query_keeps_edited_rename_and_text_units_independent() {
    let repo = TestRepo::new("list-query-rename");
    repo.write_file("src/client.rs", "header\ntimeout = 10\nfooter\n");
    repo.write_file("src/pure.rs", "unchanged\n");
    repo.write_file("tests/client.rs", "header\ntimeout = 10\nfooter\n");
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::create_dir_all(repo.path().join("archive")).unwrap();
    std::fs::rename(
        repo.path().join("src/client.rs"),
        repo.path().join("archive/client.rs"),
    )
    .unwrap();
    std::fs::rename(
        repo.path().join("src/pure.rs"),
        repo.path().join("archive/pure.rs"),
    )
    .unwrap();
    repo.write_file("archive/client.rs", "header\ntimeout = 20\nfooter\n");
    repo.write_file("tests/client.rs", "header\ntimeout = 20\nfooter\n");

    let rename = repo.hunk_ok(&["list", "--query", "renames()"]);
    let value: serde_json::Value = serde_json::from_str(&rename).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{rename}");
    assert!(files
        .iter()
        .all(|file| file["file_units"][0]["kind"] == "rename"));
    assert!(files
        .iter()
        .all(|file| file["hunks"].as_array().unwrap().is_empty()));

    let content = repo.hunk_ok(&[
        "list",
        "--query",
        "content(\"timeout\") & files(\"archive/**\")",
    ]);
    let value: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(value["files"].as_array().unwrap().len(), 1, "{content}");
    assert!(value["files"][0].get("file_units").is_none());
    assert!(value["files"][0].get("rename").is_none());
    assert_eq!(value["files"][0]["hunks"].as_array().unwrap().len(), 1);

    let combined = repo.hunk_ok(&[
        "list",
        "--query",
        "(files(\"src/**\") & renames()) | ((files(\"src/**\") | files(\"tests/**\")) & content(\"timeout\"))",
    ]);
    let value: serde_json::Value = serde_json::from_str(&combined).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 3, "{combined}");
    let archive = files
        .iter()
        .find(|file| file["path"] == "archive/client.rs")
        .unwrap();
    assert_eq!(archive["file_units"].as_array().unwrap().len(), 1);
    assert_eq!(archive["hunks"].as_array().unwrap().len(), 1);
    let occurrence_count = files
        .iter()
        .map(|file| {
            file["hunks"].as_array().unwrap().len()
                + file
                    .get("file_units")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
        })
        .sum::<usize>();
    assert_eq!(occurrence_count, 4, "{combined}");
}

#[cfg(unix)]
#[test]
fn list_query_rejects_symlinks_without_regressing_ordinary_list() {
    use std::os::unix::fs::symlink;

    let repo = TestRepo::new("list-query-symlink");
    symlink("missing-old", repo.path().join("link")).unwrap();
    repo.jj_ok(&["commit", "-m", "base"]);
    std::fs::remove_file(repo.path().join("link")).unwrap();
    symlink("missing-new", repo.path().join("link")).unwrap();

    repo.hunk_ok(&["list"]);
    let error = repo.hunk_fail(&["list", "--query", "all()"]);
    assert!(
        error.contains("does not support special-file changes"),
        "{error}"
    );
}
