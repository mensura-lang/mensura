//! The runtime configuration (ADR 0045 decision 10): a flat TOML file that
//! nothing infers, holding what an instance needs to run and nothing about
//! where.  It carries no secret: a secret-valued field takes a reference
//! to an environment variable or a file, resolved once at startup.

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// One instance's configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The program file, relative to the configuration file's directory.
    pub program: PathBuf,
    /// The database address, as a secret reference.
    pub database: SecretRef,
    /// The namespace Mensura owns in that database (decision 2).  On
    /// embedded SQLite the file is the namespace; the field is still
    /// required so that a configuration moves to Postgres unchanged.
    pub namespace: String,
    /// The roles this instance runs; all three by default (decision 1).
    #[serde(default = "all_roles")]
    pub roles: Vec<Role>,
    /// The address to bind.  A non-loopback address needs at least one
    /// principal (decision 8).
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default)]
    pub limits: Limits,
    /// The principals this instance accepts (decision 8).
    #[serde(default, rename = "principal")]
    pub principals: Vec<Principal>,
}

/// A role an instance runs (decision 1).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Intake,
    Read,
    Materializer,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Intake => "intake",
            Role::Read => "read",
            Role::Materializer => "materializer",
        }
    }
}

fn all_roles() -> Vec<Role> {
    vec![Role::Intake, Role::Read, Role::Materializer]
}

fn default_listen() -> String {
    "127.0.0.1:8080".to_string()
}

/// A reference to a secret value: exactly one of an environment variable
/// or a file (decision 10).  A literal is a load error, never a leak.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum SecretRef {
    Env { env: String },
    File { file: PathBuf },
}

impl SecretRef {
    /// Resolve the reference.  An unset variable, a missing or unreadable
    /// file, or an empty value refuses to start and names the field.
    pub fn resolve(&self, field: &str, base: &Path) -> Result<String, ConfigError> {
        let value = match self {
            SecretRef::Env { env } => std::env::var(env).map_err(|_| {
                ConfigError::Invalid(format!(
                    "`{field}` references the environment variable `{env}`, which is not set"
                ))
            })?,
            SecretRef::File { file } => {
                let path = base.join(file);
                let text = std::fs::read_to_string(&path).map_err(|e| {
                    ConfigError::Invalid(format!(
                        "`{field}` references the file {}, which cannot be read: {e}",
                        path.display()
                    ))
                })?;
                text.strip_suffix('\n').unwrap_or(&text).to_string()
            }
        };
        if value.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "`{field}` resolved to an empty value"
            )));
        }
        Ok(value)
    }
}

/// Request limits (decision 10).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// The largest request body accepted, in bytes; larger is a 413.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    /// How long a read waits for `min_position` before answering 503
    /// (decision 6), as `<n>ms`, `<n>s`, or `<n>m`.
    #[serde(default = "default_min_position_wait")]
    pub min_position_wait: String,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_body_bytes: default_max_body_bytes(),
            min_position_wait: default_min_position_wait(),
        }
    }
}

impl Limits {
    pub fn min_position_wait(&self) -> Result<Duration, ConfigError> {
        parse_duration(&self.min_position_wait).ok_or_else(|| {
            ConfigError::Invalid(format!(
                "`limits.min_position_wait` is `{}`; expected `<n>ms`, `<n>s`, or `<n>m`",
                self.min_position_wait
            ))
        })
    }
}

fn default_max_body_bytes() -> usize {
    8_388_608
}

fn default_min_position_wait() -> String {
    "5s".to_string()
}

fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    let (digits, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = digits.parse().ok()?;
    Some(match unit {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        _ => return None,
    })
}

/// A principal (decision 8): a canonical identity in ADR 0005's
/// SPIFFE-style form, the SHA-256 digest of its bearer token, and its
/// permission scopes.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub id: String,
    pub token_sha256: String,
    pub scopes: Vec<String>,
}

/// A configuration that does not load.
#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, toml::de::Error),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(path, e) => write!(f, "cannot read {}: {e}", path.display()),
            ConfigError::Parse(path, e) => {
                write!(
                    f,
                    "{} is not a valid configuration: {}",
                    path.display(),
                    e.message()
                )
            }
            ConfigError::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Load and validate a configuration file.  Relative paths in it are
    /// resolved against the file's directory.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Config::parse(&text, base).map_err(|e| match e {
            ConfigError::Parse(_, e) => ConfigError::Parse(path.to_path_buf(), e),
            other => other,
        })
    }

    /// Parse and validate configuration text, resolving relative paths
    /// against `base`.
    pub fn parse(text: &str, base: &Path) -> Result<Config, ConfigError> {
        let mut config: Config =
            toml::from_str(text).map_err(|e| ConfigError::Parse(PathBuf::from("<config>"), e))?;
        if config.program.is_relative() {
            config.program = base.join(&config.program);
        }
        if let SecretRef::File { file } = &config.database
            && file.is_relative()
        {
            config.database = SecretRef::File {
                file: base.join(file),
            };
        }
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.roles.is_empty() {
            return Err(ConfigError::Invalid(
                "`roles` is empty; an instance runs at least one role".to_string(),
            ));
        }
        for (i, role) in self.roles.iter().enumerate() {
            if self.roles[..i].contains(role) {
                return Err(ConfigError::Invalid(format!(
                    "`roles` lists `{}` twice",
                    role.name()
                )));
            }
        }
        if self.namespace.is_empty() {
            return Err(ConfigError::Invalid("`namespace` is empty".to_string()));
        }
        let addr: SocketAddr = self.listen.parse().map_err(|_| {
            ConfigError::Invalid(format!(
                "`listen` is `{}`; expected an address such as `127.0.0.1:8080`",
                self.listen
            ))
        })?;
        if !addr.ip().is_loopback() && self.principals.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "`listen` binds the non-loopback address {addr} but no principal is \
                 configured; every request but `health` needs a credential, so \
                 this would serve nothing but expose the instance (ADR 0045)"
            )));
        }
        self.limits.min_position_wait()?;
        for p in &self.principals {
            if p.token_sha256.len() != 64 || !p.token_sha256.chars().all(|c| c.is_ascii_hexdigit())
            {
                return Err(ConfigError::Invalid(format!(
                    "principal `{}`: `token_sha256` must be 64 hexadecimal characters",
                    p.id
                )));
            }
            if p.id.is_empty() {
                return Err(ConfigError::Invalid(
                    "a principal has an empty `id`".to_string(),
                ));
            }
            for scope in &p.scopes {
                let valid = scope.split_once(':').is_some_and(|(op, resource)| {
                    matches!(op, "read" | "write" | "floor") && !resource.is_empty()
                });
                if !valid {
                    return Err(ConfigError::Invalid(format!(
                        "principal `{}`: scope `{scope}` is not `read:`, `write:`, or \
                         `floor:` over a resource",
                        p.id
                    )));
                }
            }
        }
        Ok(())
    }

    /// The parsed listen address (validated at load).
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen.parse().expect("validated at load")
    }

    /// The database this instance serves, from the resolved address.
    pub fn database_target(&self, base: &Path) -> Result<DatabaseTarget, ConfigError> {
        let address = self.database.resolve("database", base)?;
        DatabaseTarget::parse(&address)
    }
}

/// Where the database is.  This slice serves embedded SQLite only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatabaseTarget {
    /// An embedded SQLite file, or `:memory:`.
    Sqlite(PathBuf),
}

impl DatabaseTarget {
    pub fn parse(address: &str) -> Result<DatabaseTarget, ConfigError> {
        if address.starts_with("postgres://") || address.starts_with("postgresql://") {
            return Err(ConfigError::Invalid(
                "`database` is a Postgres address; this build serves embedded SQLite \
                 only, and the Postgres backend is the next slice (ADR 0045)"
                    .to_string(),
            ));
        }
        if let Some((scheme, _)) = address.split_once("://") {
            return Err(ConfigError::Invalid(format!(
                "`database` uses the `{scheme}` scheme, which no backend serves"
            )));
        }
        let path = address.strip_prefix("sqlite:").unwrap_or(address);
        Ok(DatabaseTarget::Sqlite(PathBuf::from(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        program   = "fleet.mensura"
        database  = { env = "MENSURA_DATABASE_URL" }
        namespace = "fleet"
    "#;

    #[test]
    fn defaults_fill_in_and_paths_resolve_against_the_base() {
        let config = Config::parse(MINIMAL, Path::new("/etc/mensura")).unwrap();
        assert_eq!(config.program, PathBuf::from("/etc/mensura/fleet.mensura"));
        assert_eq!(config.roles, all_roles());
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert_eq!(config.limits.max_body_bytes, 8_388_608);
        assert_eq!(
            config.limits.min_position_wait().unwrap(),
            Duration::from_secs(5)
        );
        assert!(config.principals.is_empty());
    }

    #[test]
    fn a_secret_field_refuses_a_literal() {
        let text = MINIMAL.replace("{ env = \"MENSURA_DATABASE_URL\" }", "\"postgres://x\"");
        let err = Config::parse(&text, Path::new(".")).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(..)), "{err}");
        let text = MINIMAL.replace(
            "{ env = \"MENSURA_DATABASE_URL\" }",
            "{ env = \"A\", file = \"b\" }",
        );
        assert!(Config::parse(&text, Path::new(".")).is_err());
    }

    #[test]
    fn a_non_loopback_bind_needs_a_principal() {
        let text = format!("{MINIMAL}\nlisten = \"0.0.0.0:8080\"\n");
        let err = Config::parse(&text, Path::new(".")).unwrap_err();
        assert!(err.to_string().contains("no principal"), "{err}");
        let text = format!(
            "{text}\n[[principal]]\nid = \"spiffe://local/service/x\"\ntoken_sha256 = \"{}\"\nscopes = [\"read:all\"]\n",
            "0".repeat(64)
        );
        Config::parse(&text, Path::new(".")).unwrap();
    }

    #[test]
    fn scopes_and_digests_are_checked() {
        let text = format!(
            "{MINIMAL}\n[[principal]]\nid = \"spiffe://local/service/x\"\ntoken_sha256 = \"abc\"\nscopes = []\n"
        );
        assert!(Config::parse(&text, Path::new(".")).is_err());
        let text = format!(
            "{MINIMAL}\n[[principal]]\nid = \"spiffe://local/service/x\"\ntoken_sha256 = \"{}\"\nscopes = [\"admin:all\"]\n",
            "a".repeat(64)
        );
        let err = Config::parse(&text, Path::new(".")).unwrap_err();
        assert!(err.to_string().contains("admin:all"), "{err}");
    }

    #[test]
    fn the_database_address_picks_the_backend() {
        assert_eq!(
            DatabaseTarget::parse("sqlite:/var/lib/mensura/fleet.db").unwrap(),
            DatabaseTarget::Sqlite(PathBuf::from("/var/lib/mensura/fleet.db"))
        );
        assert_eq!(
            DatabaseTarget::parse("fleet.db").unwrap(),
            DatabaseTarget::Sqlite(PathBuf::from("fleet.db"))
        );
        let err = DatabaseTarget::parse("postgres://u:p@h/db").unwrap_err();
        assert!(err.to_string().contains("next slice"), "{err}");
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse_duration("250ms"), Some(Duration::from_millis(250)));
        assert_eq!(parse_duration("2m"), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration("5"), None);
        assert_eq!(parse_duration("5h"), None);
    }
}
