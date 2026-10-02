#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    bin: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        let state = root.join("state");
        let bin = root.join("bin");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&bin).unwrap();
        for (tool, var) in [("codex", "CODEX_HOME"), ("claude", "CLAUDE_CONFIG_DIR")] {
            let script = format!(
                "#!/bin/sh\nprofile_home=\"${var}\"\nprintf 'home=%s\\ncwd=%s\\npid=%s\\n' \"$profile_home\" \"$PWD\" \"$$\"\nfor arg in \"$@\"; do printf 'arg=<%s>\\n' \"$arg\"; done\nif [ \"$1\" = login ] || [ \"$1 $2\" = 'auth login' ]; then printf '%s' \"$FAKE_ACCOUNT\" > \"$profile_home/session\"; fi\nif [ -f \"$profile_home/session\" ]; then printf 'account='; cat \"$profile_home/session\"; printf '\\n'; fi\nif [ \"$FAKE_SIGNAL\" = yes ]; then kill -TERM $$; fi\nexit \"${{FAKE_EXIT:-0}}\"\n"
            );
            fs::write(bin.join(tool), script).unwrap();
            fs::set_permissions(bin.join(tool), fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            _temp: temp,
            root,
            home,
            state,
            bin,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_awitch"));
        cmd.env_clear()
            .env("HOME", &self.home)
            .env("AWITCH_HOME", &self.state)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin:/opt/homebrew/bin", self.bin.display()),
            )
            .current_dir(&self.root);
        cmd
    }

    fn ok(&self, args: &[&str]) -> String {
        success(self.command().args(args).output().unwrap())
    }

    fn init(&self) {
        self.ok(&["init"]);
        self.ok(&["profile", "add", "personal", "--default"]);
        self.ok(&["profile", "add", "work"]);
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .args(args)
            .output()
            .unwrap();
        success(output)
    }

    fn repo(&self) -> PathBuf {
        let repo = self.root.join("repo");
        fs::create_dir(&repo).unwrap();
        self.git(&repo, &["init", "-q"]);
        self.git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:Example/project.git",
            ],
        );
        repo
    }
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn failure(output: Output, message: &str) {
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn setup_login_and_run_isolate_both_tools_and_preserve_arguments() {
    let f = Fixture::new();
    f.init();
    assert_eq!(f.ok(&["profile", "list"]), "personal (default)\nwork\n");
    for tool in ["codex", "claude"] {
        for profile in ["personal", "work"] {
            let account = format!("{profile}-{tool}");
            let output = success(
                f.command()
                    .env("FAKE_ACCOUNT", &account)
                    .args(["login", profile, tool])
                    .output()
                    .unwrap(),
            );
            assert!(output.contains("arg=<login>"));
            assert_eq!(output.contains("arg=<auth>"), tool == "claude");
            let home = f.state.join("profiles").join(profile).join(tool);
            assert_eq!(fs::read_to_string(home.join("session")).unwrap(), account);
            assert_eq!(
                fs::metadata(&home).unwrap().permissions().mode() & 0o777,
                0o700
            );
            let output = success(
                f.command()
                    .env("CODEX_HOME", "/must-not-be-used")
                    .env("CLAUDE_CONFIG_DIR", "/must-not-be-used")
                    .args([
                        "--profile",
                        profile,
                        tool,
                        "--help",
                        "two words",
                        "$(echo unsafe)",
                        "--profile",
                        "cli-profile",
                    ])
                    .output()
                    .unwrap(),
            );
            assert!(output.contains(&format!("home={}\n", home.display())));
            assert!(output.contains(&format!("account={account}\n")));
            assert!(output.contains("arg=<--help>\narg=<two words>\narg=<$(echo unsafe)>\narg=<--profile>\narg=<cli-profile>\n"));
        }
    }
    assert_eq!(
        fs::metadata(f.state.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let output = f
        .command()
        .env("FAKE_EXIT", "42")
        .args(["codex"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42));
    let output = f
        .command()
        .env("FAKE_SIGNAL", "yes")
        .args(["claude"])
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(15));
}

#[test]
fn owner_routing_from_subdirectory_and_linked_worktree() {
    let f = Fixture::new();
    f.init();
    let repo = f.repo();
    let sub = repo.join("src");
    fs::create_dir(&sub).unwrap();
    f.ok(&["rule", "add", "work", "--org", "eXample"]);
    let output = success(f.command().current_dir(&sub).arg("which").output().unwrap());
    assert!(output.contains("profile: work\nreason: rule 1"));
    assert!(output.contains("org: example"));
    f.git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    );
    let linked = f.root.join("linked");
    f.git(
        &repo,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let output = success(
        f.command()
            .current_dir(&linked)
            .arg("which")
            .output()
            .unwrap(),
    );
    assert!(output.contains("profile: work"));
    f.ok(&[
        "rule",
        "add",
        "personal",
        "--path",
        linked.to_str().unwrap(),
    ]);
    let output = success(
        f.command()
            .args(["--cwd", linked.to_str().unwrap(), "codex"])
            .output()
            .unwrap(),
    );
    assert!(output.contains(&format!("cwd={}\n", linked.display())));
    assert!(output.contains("profiles/personal/codex"));
    f.git(
        &repo,
        &[
            "remote",
            "set-url",
            "origin",
            "https://gitlab.com/example/project",
        ],
    );
    assert!(
        success(
            f.command()
                .current_dir(&repo)
                .arg("which")
                .output()
                .unwrap()
        )
        .contains("org: (unknown)")
    );
}

#[test]
fn absolute_cwd_recovers_from_deleted_working_directory() {
    let f = Fixture::new();
    f.init();
    let repo = f.repo();
    f.ok(&["rule", "add", "work", "--path", repo.to_str().unwrap()]);
    let deleted = f.root.join("deleted");
    fs::create_dir(&deleted).unwrap();
    let command = f.command();
    let output = Command::new("/bin/sh")
        .env_clear()
        .envs(
            command
                .get_envs()
                .map(|(name, value)| (name, value.unwrap())),
        )
        .current_dir(&deleted)
        .args(["-c", "rmdir \"$1\" && shift && exec \"$@\"", "sh"])
        .arg(&deleted)
        .arg(env!("CARGO_BIN_EXE_awitch"))
        .args(["--cwd", repo.to_str().unwrap(), "codex"])
        .output()
        .unwrap();
    assert!(!deleted.exists());
    let output = success(output);
    assert!(output.contains(&format!("cwd={}\n", repo.display())));
    assert!(output.contains(&format!(
        "home={}\n",
        f.state.join("profiles/work/codex").display()
    )));
}

#[test]
fn canonical_paths_tilde_boundaries_and_org_and_path() {
    let f = Fixture::new();
    f.init();
    let repo = f.repo();
    let alias = f.home.join("alias");
    symlink(&repo, &alias).unwrap();
    f.ok(&[
        "rule", "add", "work", "--path", "~/alias", "--org", "example",
    ]);
    assert!(
        f.ok(&["rule", "list"])
            .contains(&format!("path={}", repo.display()))
    );
    assert!(
        success(
            f.command()
                .args(["--cwd", alias.to_str().unwrap(), "which"])
                .output()
                .unwrap()
        )
        .contains("profile: work")
    );
    f.git(&repo, &["remote", "remove", "origin"]);
    assert!(
        success(
            f.command()
                .current_dir(&repo)
                .arg("which")
                .output()
                .unwrap()
        )
        .contains("profile: personal")
    );
    f.ok(&["rule", "add", "work", "--path", repo.to_str().unwrap()]);
    let sibling = f.root.join("repo-other");
    fs::create_dir(&sibling).unwrap();
    assert!(
        success(
            f.command()
                .current_dir(&sibling)
                .arg("which")
                .output()
                .unwrap()
        )
        .contains("profile: personal")
    );
    let obsolete = f.root.join("obsolete");
    let previous = obsolete.join("project");
    fs::create_dir_all(&previous).unwrap();
    f.ok(&["rule", "add", "work", "--path", previous.to_str().unwrap()]);
    fs::remove_dir_all(&obsolete).unwrap();
    fs::write(&obsolete, "replaced directory").unwrap();
    assert!(f.ok(&["which"]).contains("profile: personal"));
}

#[test]
fn ambiguous_and_unmatched_rules_fail_without_launching() {
    let f = Fixture::new();
    f.ok(&["init"]);
    f.ok(&["profile", "add", "personal"]);
    f.ok(&["profile", "add", "work"]);
    failure(
        f.command().arg("codex").output().unwrap(),
        "no matching profile",
    );
    for profile in ["personal", "work"] {
        f.ok(&["rule", "add", profile, "--path", f.root.to_str().unwrap()]);
    }
    failure(f.command().arg("claude").output().unwrap(), "ambiguous");
    assert!(
        f.ok(&["--profile", "work", "which"])
            .contains("reason: explicit")
    );
    assert!(!f.state.join("profiles").exists());
}

#[test]
fn validation_keeps_config_and_credentials_untouched() {
    let f = Fixture::new();
    f.init();
    let before = fs::read(f.state.join("config.toml")).unwrap();
    for args in [
        vec!["init"],
        vec!["profile", "add", "../escape"],
        vec!["profile", "add", "WORK"],
        vec!["profile", "add", "work"],
        vec!["rule", "add", "missing", "--org", "example"],
        vec!["rule", "add", "work", "--org", "example/repo"],
    ] {
        assert!(!f.command().args(args).output().unwrap().status.success());
        assert_eq!(fs::read(f.state.join("config.toml")).unwrap(), before);
    }
    failure(
        f.command()
            .args(["login", "unknown", "codex"])
            .output()
            .unwrap(),
        "unknown profile",
    );
    failure(
        f.command().args(["codex", "--cd=/tmp"]).output().unwrap(),
        "--cwd",
    );
    failure(
        f.command().args(["codex", "-C/tmp"]).output().unwrap(),
        "--cwd",
    );
    assert!(!f.state.join("profiles").exists());
    fs::write(f.state.join("config.toml"), "version = 999\n").unwrap();
    failure(
        f.command().arg("which").output().unwrap(),
        "unsupported config version",
    );
    fs::write(f.state.join("config.toml"), "typo = true\n").unwrap();
    failure(f.command().arg("which").output().unwrap(), "unknown field");
}

#[test]
fn conflicting_auth_environment_is_rejected_without_exposing_values() {
    let f = Fixture::new();
    f.init();
    for (tool, variable) in [
        ("codex", "OPENAI_API_KEY"),
        ("codex", "OPENAI_FEDERATION_RULE_ID"),
        ("codex", "OPENAI_IDENTITY_TOKEN_FILE"),
        ("claude", "CLAUDE_CODE_OAUTH_TOKEN"),
        ("claude", "CLAUDE_SECURESTORAGE_CONFIG_DIR"),
        ("claude", "CLAUDE_CODE_USE_ANTHROPIC_AWS"),
    ] {
        let output = f
            .command()
            .env(variable, "never-print-this-secret")
            .arg(tool)
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&output.stderr).contains("never-print-this-secret"));
        failure(output, variable);
    }
    assert!(!f.state.join("profiles").exists());
}

#[test]
fn concurrent_config_updates_do_not_lose_profiles() {
    let f = Fixture::new();
    f.ok(&["init"]);
    let mut children = Vec::new();
    for i in 0..12 {
        children.push(
            f.command()
                .args(["profile", "add", &format!("p{i}")])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        success(child.wait_with_output().unwrap());
    }
    assert_eq!(f.ok(&["profile", "list"]).lines().count(), 12);
}

#[test]
fn managed_symlinks_are_not_followed() {
    let f = Fixture::new();
    f.init();
    let external = f.root.join("external");
    fs::create_dir(&external).unwrap();
    symlink(&external, f.state.join("profiles")).unwrap();
    failure(
        f.command().arg("codex").output().unwrap(),
        "must not be a symlink",
    );
    assert_eq!(fs::read_dir(external).unwrap().count(), 0);
}

#[test]
fn inherited_git_directory_cannot_override_cwd_routing() {
    let f = Fixture::new();
    f.init();
    let repo = f.repo();
    f.ok(&["rule", "add", "work", "--org", "example"]);
    let output = success(
        f.command()
            .env("GIT_DIR", repo.join(".git"))
            .env("GIT_WORK_TREE", repo)
            .arg("which")
            .output()
            .unwrap(),
    );
    assert!(output.contains("profile: personal"));
}

#[test]
fn config_location_precedence_and_missing_cli() {
    let f = Fixture::new();
    let xdg = f.root.join("xdg");
    success(
        f.command()
            .env_remove("AWITCH_HOME")
            .env("XDG_CONFIG_HOME", &xdg)
            .arg("init")
            .output()
            .unwrap(),
    );
    assert!(xdg.join("awitch/config.toml").is_file());
    success(
        f.command()
            .env_remove("AWITCH_HOME")
            .arg("init")
            .output()
            .unwrap(),
    );
    assert!(f.home.join(".config/awitch/config.toml").is_file());
    success(
        f.command()
            .env("XDG_CONFIG_HOME", &xdg)
            .arg("init")
            .output()
            .unwrap(),
    );
    assert!(f.state.join("config.toml").is_file());
    f.ok(&["profile", "add", "personal", "--default"]);
    fs::remove_file(f.bin.join("codex")).unwrap();
    let output = f
        .command()
        .env("PATH", &f.bin)
        .arg("codex")
        .output()
        .unwrap();
    failure(output, "cannot launch codex");
    assert!(
        success(
            f.command()
                .env("PATH", &f.bin)
                .arg("which")
                .output()
                .unwrap()
        )
        .contains("profile: personal")
    );
}

#[test]
fn simultaneous_profiles_have_independent_sessions_and_exec_preserves_pid() {
    let f = Fixture::new();
    f.init();
    let mut children = Vec::new();
    for tool in ["codex", "claude"] {
        for profile in ["personal", "work"] {
            let child = f
                .command()
                .env("FAKE_ACCOUNT", profile)
                .args(["login", profile, tool])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let pid = child.id();
            children.push((profile, tool, pid, child));
        }
    }
    for (profile, tool, pid, child) in children {
        let output = success(child.wait_with_output().unwrap());
        assert!(output.contains(&format!("pid={pid}\n")));
        assert_eq!(
            fs::read_to_string(
                f.state
                    .join("profiles")
                    .join(profile)
                    .join(tool)
                    .join("session")
            )
            .unwrap(),
            profile
        );
    }
}
