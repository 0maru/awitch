use crate::config::Config;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn github_owner(remote: &str) -> Option<String> {
    let (host, path) = if let Some((scheme, rest)) = remote.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        (host, path)
    } else {
        let (authority, path) = remote.split_once(':')?;
        (authority.rsplit('@').next()?, path)
    };
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let mut parts = path.trim_end_matches('/').split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if crate::config::valid_org(owner).is_err() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(owner.to_ascii_lowercase())
}

pub fn detect_owner(cwd: &Path, remote: &str) -> Result<Option<String>> {
    let output = match Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .arg("-C")
        .arg(cwd)
        .args(["remote", "get-url", remote])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("cannot run git"),
    };
    if !output.status.success() {
        return Ok(None);
    }
    // remote に埋め込まれた認証情報は診断にも表示しない。
    Ok(std::str::from_utf8(&output.stdout)
        .ok()
        .and_then(|s| github_owner(s.trim())))
}

#[derive(Debug)]
pub struct Selection {
    pub profile: String,
    pub reason: String,
}

pub fn select(
    config: &Config,
    cwd: &Path,
    org: Option<&str>,
    explicit: Option<&str>,
) -> Result<Selection> {
    if let Some(profile) = explicit {
        config.require_profile(profile)?;
        return Ok(Selection {
            profile: profile.into(),
            reason: "explicit --profile".into(),
        });
    }
    let mut candidates = Vec::new();
    for (index, rule) in config.rules.iter().enumerate() {
        if let Some(expected) = &rule.org
            && !org.is_some_and(|org| org.eq_ignore_ascii_case(expected))
        {
            continue;
        }
        let depth = if let Some(path) = &rule.path {
            let path = match path.canonicalize() {
                Ok(path) => path,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error).context("cannot resolve rule path"),
            };
            if !cwd.starts_with(&path) {
                continue;
            }
            path.components().count()
        } else {
            0
        };
        candidates.push(((depth, rule.org.is_some()), index, &rule.profile));
    }
    if let Some((rank, index, profile)) = candidates.iter().max_by_key(|(rank, _, _)| rank) {
        if candidates
            .iter()
            .any(|(other_rank, _, other)| other_rank == rank && other != profile)
        {
            bail!("ambiguous matching rules; use --profile or adjust config.toml");
        }
        return Ok(Selection {
            profile: (*profile).clone(),
            reason: format!("rule {}", index + 1),
        });
    }
    if let Some(profile) = &config.default {
        return Ok(Selection {
            profile: profile.clone(),
            reason: "default".into(),
        });
    }
    bail!("no matching profile; add a rule, set a default, or use --profile")
}

pub fn working_dir(path: Option<PathBuf>) -> Result<PathBuf> {
    crate::config::directory(&path.unwrap_or(std::env::current_dir()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rule;

    #[test]
    fn github_urls_and_unrelated_hosts() {
        for url in [
            "git@github.com:Example/repo.git",
            "https://github.com/Example/repo.git",
            "ssh://git@github.com:22/Example/repo.git",
            "git://github.com/Example/repo/",
            "https://user:secret@GitHub.com/EXAMPLE/repo",
        ] {
            assert_eq!(github_owner(url).as_deref(), Some("example"), "{url}");
        }
        for url in [
            "git@gitlab.com:Example/repo.git",
            "https://github.com.evil.test/Example/repo",
            "https://github.com@evil.test/Example/repo",
            "https://github.com/Example",
            "https://github.com/Example/repo/extra",
            "/local/Example/repo",
            "github-work:Example/repo",
            "file://github.com/Example/repo",
        ] {
            assert_eq!(github_owner(url), None, "{url}");
        }
    }

    #[test]
    fn precedence_boundaries_ambiguity_and_missing_routes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let nested = root.join("work/project");
        let sibling = root.join("work-other");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let mut config = Config {
            profiles: ["personal", "org", "path", "both", "deep"]
                .into_iter()
                .map(String::from)
                .collect(),
            default: Some("personal".into()),
            ..Config::default()
        };
        let rule = |profile: &str, org: Option<&str>, path: Option<PathBuf>| Rule {
            profile: profile.into(),
            org: org.map(String::from),
            path,
        };
        config.rules = vec![
            rule("org", Some("Example"), None),
            rule("path", None, Some(root.join("work"))),
            rule("both", Some("example"), Some(root.join("work"))),
            rule("deep", None, Some(nested.clone())),
        ];
        let choose =
            |config: &Config, cwd: &Path, org| select(config, cwd, org, None).unwrap().profile;
        assert_eq!(choose(&config, &nested, Some("EXAMPLE")), "deep");
        assert_eq!(choose(&config, &root.join("work"), Some("example")), "both");
        assert_eq!(choose(&config, &root.join("work"), None), "path");
        assert_eq!(choose(&config, &sibling, Some("example")), "org");
        assert_eq!(choose(&config, &sibling, None), "personal");
        assert_eq!(
            select(&config, &nested, Some("example"), Some("personal"))
                .unwrap()
                .profile,
            "personal"
        );
        assert!(select(&config, &nested, None, Some("unknown")).is_err());
        config
            .rules
            .push(rule("personal", None, Some(nested.clone())));
        assert!(
            select(&config, &nested, None, None)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        config.rules.pop();
        config.rules.push(rule("deep", None, Some(nested.clone())));
        assert_eq!(choose(&config, &nested, None), "deep");
        config.default = None;
        assert!(select(&config, &sibling, None, None).is_err());
    }
}
