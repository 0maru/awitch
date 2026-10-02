#[cfg(not(unix))]
compile_error!("awitch currently supports macOS and Linux");

mod config;
mod launch;
mod routing;

use anyhow::{Result, ensure};
use clap::{Args, Parser, Subcommand};
use config::{Rule, Store};
use launch::Tool;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Route Claude Code and Codex accounts by GitHub owner and directory"
)]
struct Cli {
    /// プロファイルを明示指定（サブコマンドの前に置く）
    #[arg(long)]
    profile: Option<String>,
    /// 選択と起動に使う作業ディレクトリ
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// 空の設定を作成する
    Init,
    /// アカウントプロファイルを管理する
    Profile {
        #[command(subcommand)]
        command: ProfileAction,
    },
    /// 選択ルールを管理する
    Rule {
        #[command(subcommand)]
        command: RuleAction,
    },
    /// 選択されるプロファイルと保存先を確認する
    Which,
    /// 指定プロファイルで各 CLI のログインを実行する
    #[command(disable_help_flag = true)]
    Login {
        profile: String,
        tool: Tool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// 選択したプロファイルで Codex を起動する（以降は Codex の引数）
    #[command(disable_help_flag = true)]
    Codex(Passthrough),
    /// 選択したプロファイルで Claude Code を起動する（以降は Claude の引数）
    #[command(disable_help_flag = true)]
    Claude(Passthrough),
}

#[derive(Args)]
struct Passthrough {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<OsString>,
}

#[derive(Subcommand)]
enum ProfileAction {
    /// プロファイルを登録する（ログインは各 CLI で別途実行）
    Add {
        name: String,
        #[arg(long)]
        default: bool,
    },
    /// プロファイルとデフォルトを一覧表示する
    List,
}

#[derive(Subcommand)]
enum RuleAction {
    /// org・パス・両方の AND 条件でルールを追加する
    Add {
        profile: String,
        #[arg(long, required_unless_present = "path")]
        org: Option<String>,
        #[arg(long, required_unless_present = "org")]
        path: Option<PathBuf>,
    },
    /// ルールを設定順に一覧表示する（which と共通の番号）
    List,
}

fn run() -> Result<()> {
    let raw_args: Vec<_> = std::env::args_os().collect();
    let mut cli = Cli::parse_from(&raw_args);
    let forwarded_args = match &mut cli.command {
        Action::Codex(args) | Action::Claude(args) => Some(&mut args.args),
        Action::Login { args, .. } => Some(args),
        _ => None,
    };
    if let Some(args) = forwarded_args {
        // Clap が転送引数の先頭で消費した区切りだけを元の引数列から戻す。
        if raw_args[raw_args.len() - args.len() - 1] == "--" {
            args.insert(0, OsString::from("--"));
        }
    }
    let store = Store::from_env()?;
    if cli.profile.is_some() {
        ensure!(
            matches!(
                cli.command,
                Action::Which | Action::Codex(_) | Action::Claude(_)
            ),
            "--profile is only valid for which, codex and claude"
        );
    }
    let cwd = routing::working_dir(cli.cwd)?;
    match cli.command {
        Action::Init => {
            store.update(true, |_| Ok(()))?;
            println!("Created {}", store.path().display());
        }
        Action::Profile {
            command: ProfileAction::Add { name, default },
        } => {
            config::valid_name(&name)?;
            store.update(false, |config| {
                ensure!(
                    config.profiles.insert(name.clone()),
                    "profile already exists: {name}"
                );
                if default {
                    config.default = Some(name.clone());
                }
                Ok(())
            })?;
            println!("Added profile {name}");
        }
        Action::Profile {
            command: ProfileAction::List,
        } => {
            let config = store.load()?;
            for name in &config.profiles {
                println!(
                    "{name}{}",
                    if config.default.as_ref() == Some(name) {
                        " (default)"
                    } else {
                        ""
                    }
                );
            }
        }
        Action::Rule {
            command: RuleAction::Add { profile, org, path },
        } => {
            let path = path
                .map(|path| config::directory(&cwd.join(config::expand_path(&path)?)))
                .transpose()?;
            let rule = Rule {
                profile,
                org: org.map(|s| s.to_ascii_lowercase()),
                path,
            };
            store.update(false, |config| {
                ensure!(!config.rules.contains(&rule), "rule already exists");
                config.rules.push(rule);
                Ok(())
            })?;
            println!("Added rule");
        }
        Action::Rule {
            command: RuleAction::List,
        } => {
            for (index, rule) in store.load()?.rules.iter().enumerate() {
                println!(
                    "{}: {} org={} path={}",
                    index + 1,
                    rule.profile,
                    rule.org.as_deref().unwrap_or("*"),
                    rule.path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .as_deref()
                        .unwrap_or("*")
                );
            }
        }
        Action::Login {
            profile,
            tool,
            args,
        } => {
            store.load()?.require_profile(&profile)?;
            launch::launch(&store, &profile, tool, &cwd, args, true)?;
        }
        action => {
            let config = store.load()?;
            let org = routing::detect_owner(&cwd, &config.remote)?;
            let selected = routing::select(&config, &cwd, org.as_deref(), cli.profile.as_deref())?;
            match action {
                Action::Which => {
                    println!(
                        "profile: {}\nreason: {}\ncwd: {}\norg: {}",
                        selected.profile,
                        selected.reason,
                        cwd.display(),
                        org.as_deref().unwrap_or("(unknown)")
                    );
                    for tool in [Tool::Codex, Tool::Claude] {
                        println!(
                            "{}: {}",
                            tool.home_var(),
                            store
                                .root
                                .join("profiles")
                                .join(&selected.profile)
                                .join(tool.name())
                                .display()
                        );
                    }
                }
                Action::Codex(args) => launch::launch(
                    &store,
                    &selected.profile,
                    Tool::Codex,
                    &cwd,
                    args.args,
                    false,
                )?,
                Action::Claude(args) => launch::launch(
                    &store,
                    &selected.profile,
                    Tool::Claude,
                    &cwd,
                    args.args,
                    false,
                )?,
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("awitch: {error:#}");
        std::process::exit(1);
    }
}
