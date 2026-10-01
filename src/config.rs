use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub remote: String,
    pub default: Option<String>,
    pub profiles: BTreeSet<String>,
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            remote: "origin".into(),
            default: None,
            profiles: BTreeSet::new(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub profile: String,
    pub org: Option<String>,
    pub path: Option<PathBuf>,
}

pub fn valid_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "profile name must be 1-64 ASCII letters, digits, '-' or '_', starting with a letter or digit"
    );
    Ok(())
}

pub fn valid_org(org: &str) -> Result<()> {
    ensure!(
        !org.is_empty() && org.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "org must be a GitHub owner name (letters, digits or '-')"
    );
    Ok(())
}

impl Config {
    pub fn require_profile(&self, name: &str) -> Result<()> {
        valid_name(name)?;
        ensure!(self.profiles.contains(name), "unknown profile: {name}");
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported config version: {}",
            self.version
        );
        ensure!(
            !self.remote.is_empty()
                && !self.remote.starts_with('-')
                && !self.remote.contains(char::is_whitespace),
            "invalid remote name"
        );
        let mut names = BTreeSet::new();
        for name in &self.profiles {
            valid_name(name)?;
            // macOS の大文字小文字を区別しないファイルシステムでも衝突させない。
            ensure!(
                names.insert(name.to_ascii_lowercase()),
                "profile names differ only by case"
            );
        }
        if let Some(name) = &self.default {
            self.require_profile(name)?;
        }
        for rule in &self.rules {
            self.require_profile(&rule.profile)?;
            ensure!(
                rule.org.is_some() || rule.path.is_some(),
                "rule requires org or path"
            );
            if let Some(org) = &rule.org {
                valid_org(org)?;
            }
            if let Some(path) = &rule.path {
                ensure!(
                    path.is_absolute(),
                    "rule path must be absolute: {}",
                    path.display()
                );
                ensure!(
                    !path
                        .components()
                        .any(|c| c == std::path::Component::ParentDir),
                    "rule path must not contain '..'"
                );
            }
        }
        Ok(())
    }
}

pub fn expand_path(path: &Path) -> Result<PathBuf> {
    if let Ok(rest) = path.strip_prefix("~") {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        return Ok(PathBuf::from(home).join(rest));
    }
    Ok(path.to_path_buf())
}

pub fn directory(path: &Path) -> Result<PathBuf> {
    let path = expand_path(path)?;
    let path = path
        .canonicalize()
        .with_context(|| format!("cannot resolve directory {}", path.display()))?;
    ensure!(path.is_dir(), "not a directory: {}", path.display());
    Ok(path)
}

pub fn private_dir(path: &Path) -> Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "managed directory must not be a symlink: {}",
        path.display()
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn from_env() -> Result<Self> {
        let root = if let Some(root) = std::env::var_os("AWITCH_HOME") {
            ensure!(!root.is_empty(), "AWITCH_HOME must not be empty");
            PathBuf::from(root)
        } else if let Some(root) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            let root = PathBuf::from(root);
            ensure!(root.is_absolute(), "XDG_CONFIG_HOME must be absolute");
            root.join("awitch")
        } else {
            PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
                .join(".config/awitch")
        };
        let root = expand_path(&root)?;
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()?.join(root)
        };
        Ok(Self { root })
    }

    pub fn path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn load(&self) -> Result<Config> {
        let path = self.path();
        let meta =
            fs::symlink_metadata(&path).context("config missing; run `awitch init` first")?;
        ensure!(meta.is_file(), "config.toml must be a regular file");
        let config: Config = toml::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("invalid config: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn update(
        &self,
        initialize: bool,
        edit: impl FnOnce(&mut Config) -> Result<()>,
    ) -> Result<()> {
        private_dir(&self.root)?;
        let lock_path = self.root.join("config.lock");
        if let Ok(meta) = fs::symlink_metadata(&lock_path) {
            ensure!(meta.is_file(), "config.lock must be a regular file");
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(lock_path)?;
        lock.lock()?;
        let mut config = if initialize {
            if fs::symlink_metadata(self.path()).is_ok() {
                bail!("config already exists: {}", self.path().display());
            }
            Config::default()
        } else {
            self.load()?
        };
        edit(&mut config)?;
        config.validate()?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temp.write_all(toml::to_string_pretty(&config)?.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(self.path())?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    pub fn tool_home(&self, profile: &str, tool: &str) -> Result<PathBuf> {
        valid_name(profile)?;
        let mut path = self.root.clone();
        private_dir(&path)?;
        for part in ["profiles", profile, tool] {
            path.push(part);
            private_dir(&path)?;
        }
        directory(&path)
    }
}
