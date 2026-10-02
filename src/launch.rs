use crate::config::Store;
use anyhow::{Context, Result, bail, ensure};
use clap::ValueEnum;
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Tool {
    Codex,
    Claude,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub fn home_var(self) -> &'static str {
        match self {
            Self::Codex => "CODEX_HOME",
            Self::Claude => "CLAUDE_CONFIG_DIR",
        }
    }

    pub fn conflicts(self) -> &'static [&'static str] {
        match self {
            Self::Codex => &[
                "OPENAI_API_KEY",
                "CODEX_API_KEY",
                "CODEX_ACCESS_TOKEN",
                "OPENAI_BASE_URL",
                "OPENAI_FEDERATION_RULE_ID",
                "OPENAI_IDENTITY_TOKEN_FILE",
            ],
            Self::Claude => &[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
                "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
                "CLAUDE_SECURESTORAGE_CONFIG_DIR",
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_PROFILE",
                "ANTHROPIC_CONFIG_DIR",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
                "CLAUDE_CODE_USE_ANTHROPIC_AWS",
            ],
        }
    }
}

pub fn validate_env(tool: Tool) -> Result<()> {
    let conflicts: Vec<_> = tool
        .conflicts()
        .iter()
        .filter(|name| std::env::var_os(name).is_some())
        .copied()
        .collect();
    ensure!(
        conflicts.is_empty(),
        "unset authentication/provider overrides before using awitch: {}",
        conflicts.join(", ")
    );
    Ok(())
}

pub fn launch(
    store: &Store,
    profile: &str,
    tool: Tool,
    cwd: &Path,
    args: Vec<OsString>,
    login: bool,
) -> Result<()> {
    validate_env(tool)?;
    if matches!(tool, Tool::Codex) {
        for arg in args.iter().take_while(|arg| *arg != "--") {
            let arg = arg.to_string_lossy();
            if arg == "--cd" || arg.starts_with("--cd=") || arg.starts_with("-C") {
                bail!(
                    "use `awitch --cwd DIR codex ...` instead of Codex -C/--cd so routing uses the target directory"
                );
            }
        }
    }
    let home = store.tool_home(profile, tool.name())?;
    let mut command = Command::new(tool.name());
    command.current_dir(cwd).env(tool.home_var(), home);
    if login {
        match tool {
            Tool::Codex => {
                command.arg("login");
            }
            Tool::Claude => {
                command.args(["auth", "login"]);
            }
        }
    }
    command.args(args);
    Err(command.exec()).with_context(|| {
        format!(
            "cannot launch {}; install it and make it available on PATH",
            tool.name()
        )
    })
}
