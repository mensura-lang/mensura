//! The deploy configuration (ADR 0046 decision 2): a TOML file the user
//! writes beside the program, naming the program, the namespace, the
//! target, the database, the instances, and the principals.  Nothing in
//! it is inferred from the host, and nothing in it is secret: a token
//! appears only as its digest.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;

use mensura_serve::Role;
use mensura_serve::config::{Limits, Principal};
use serde::Deserialize;

use crate::DeployError;

/// The deploy configuration, as written.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployConfig {
    /// The program file, relative to the configuration file.
    pub program: PathBuf,
    /// ADR 0045 decision 2's namespace; on SQLite it names the database
    /// file on the data volume.
    pub namespace: String,
    pub target: Target,
    pub database: Database,
    /// The `serve` processes; at least one.
    #[serde(rename = "instance")]
    pub instances: Vec<Instance>,
    /// Passed through unchanged into every runtime configuration.
    #[serde(default)]
    pub limits: Limits,
    /// Passed through unchanged into every runtime configuration.
    #[serde(default, rename = "principal")]
    pub principals: Vec<Principal>,
}

/// Where the bundle runs (decision 2): the kind, the project name the
/// target's tooling uses, and the container image (decision 4).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub kind: String,
    pub name: String,
    pub image: String,
}

/// The database kind; `sqlite` is the only kind of this slice.  Required,
/// like the instances, their roles, and their addresses: each decides
/// what runs and who can reach it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    pub kind: String,
}

/// One `serve` process (decision 2).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub name: String,
    pub roles: Vec<Role>,
    /// The host address and port the listener is published on.
    pub publish: String,
    /// A compose memory limit such as `256M`; none when absent.
    pub memory: Option<String>,
}

impl Instance {
    /// The parsed publish address (validated by [`DeployConfig::parse`]).
    pub fn publish_addr(&self) -> SocketAddr {
        self.publish.parse().expect("validated at parse")
    }
}

impl DeployConfig {
    /// Parse and validate configuration text.  The program path is left
    /// relative; the caller resolves it against the file's directory.
    pub fn parse(text: &str) -> Result<DeployConfig, DeployError> {
        let config: DeployConfig =
            toml::from_str(text).map_err(|e| DeployError::Invalid(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), DeployError> {
        let invalid = |msg: String| Err(DeployError::Invalid(msg));
        if self.program.file_name().is_none() {
            return invalid(format!(
                "`program` is `{}`, which names no file",
                self.program.display()
            ));
        }
        if !is_file_safe(&self.namespace) {
            return invalid(format!(
                "`namespace` is `{}`; it names the database file, so it must be \
                 non-empty and use only ASCII letters, digits, `-`, and `_`",
                self.namespace
            ));
        }
        match self.target.kind.as_str() {
            "compose" => {}
            other => {
                return invalid(format!(
                    "`target.kind` is `{other}`; `compose` is the only target \
                     of this release (ADR 0046)"
                ));
            }
        }
        if !is_service_name(&self.target.name) {
            return invalid(format!(
                "`target.name` is `{}`; a compose project name uses only lowercase \
                 ASCII letters, digits, `-`, and `_`, and starts with a letter or digit",
                self.target.name
            ));
        }
        if self.target.image.is_empty() || self.target.image.chars().any(char::is_whitespace) {
            return invalid(format!(
                "`target.image` is `{}`; it must name a container image, \
                 ideally pinned by digest or commit tag (ADR 0046 decision 4)",
                self.target.image
            ));
        }
        match self.database.kind.as_str() {
            "sqlite" => {}
            "postgres" => {
                return invalid(
                    "`database.kind` is `postgres`; this release deploys embedded \
                     SQLite only, and the Postgres target arrives with the Postgres \
                     backend (ADR 0046)"
                        .to_string(),
                );
            }
            other => {
                return invalid(format!(
                    "`database.kind` is `{other}`; the only kind is `sqlite`"
                ));
            }
        }
        if self.instances.is_empty() {
            return invalid("`instance` is empty; declare at least one `[[instance]]`".to_string());
        }
        let mut names = BTreeSet::new();
        let mut published = BTreeSet::new();
        for instance in &self.instances {
            if !is_service_name(&instance.name) {
                return invalid(format!(
                    "instance `{}`: a name uses only lowercase ASCII letters, digits, \
                     `-`, and `_`, and starts with a letter or digit, since it names \
                     the compose service and the runtime configuration file",
                    instance.name
                ));
            }
            if !names.insert(instance.name.as_str()) {
                return invalid(format!("instance `{}` is declared twice", instance.name));
            }
            if instance.roles.is_empty() {
                return invalid(format!(
                    "instance `{}`: `roles` is empty; an instance runs at least one role",
                    instance.name
                ));
            }
            for (i, role) in instance.roles.iter().enumerate() {
                if instance.roles[..i].contains(role) {
                    return invalid(format!(
                        "instance `{}`: `roles` lists `{}` twice",
                        instance.name,
                        role.name()
                    ));
                }
            }
            let addr: SocketAddr = instance.publish.parse().map_err(|_| {
                DeployError::Invalid(format!(
                    "instance `{}`: `publish` is `{}`; expected a host address and \
                     port such as `127.0.0.1:8080`",
                    instance.name, instance.publish
                ))
            })?;
            if !published.insert(addr) {
                return invalid(format!(
                    "instance `{}`: `publish` address {addr} is already taken by \
                     another instance",
                    instance.name
                ));
            }
            if let Some(memory) = &instance.memory
                && !is_byte_value(memory)
            {
                return invalid(format!(
                    "instance `{}`: `memory` is `{memory}`; expected a size such as \
                     `256M` or `1G`",
                    instance.name
                ));
            }
        }
        // Embedded SQLite is one file in one process: the all-in-one
        // deployment only (ADR 0045 decision 2), refused here exactly as
        // `serve` would refuse it at start.
        if self.instances.len() > 1 {
            return invalid(format!(
                "{} instances are declared, but embedded SQLite serves only the \
                 all-in-one deployment, one instance running every role; \
                 separating roles across instances needs a database reached by \
                 address (ADR 0045 decision 2)",
                self.instances.len()
            ));
        }
        let only = &self.instances[0];
        if only.roles.len() < 3 {
            return invalid(format!(
                "instance `{}` is configured with {} of the three roles, but \
                 embedded SQLite serves only the all-in-one deployment; \
                 separating roles across instances needs a database reached by \
                 address (ADR 0045 decision 2)",
                only.name,
                only.roles.len()
            ));
        }
        // Inside the container the listener binds every interface, which
        // ADR 0045 decision 8 admits only with a principal.
        if self.principals.is_empty() {
            return invalid(
                "no `[[principal]]` is configured; a deployed instance listens on \
                 every interface inside its container, which ADR 0045 admits only \
                 with a principal, and with none it would serve nothing but \
                 `health`.  `mensura token` generates a token and its digest"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// A namespace usable as a file name on every host.
fn is_file_safe(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A compose project or service name, also used as a file name.
fn is_service_name(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// A compose byte value: digits and an optional unit.
fn is_byte_value(s: &str) -> bool {
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (digits, unit) = s.split_at(split);
    !digits.is_empty()
        && matches!(
            unit.to_ascii_lowercase().as_str(),
            "" | "b" | "k" | "kb" | "m" | "mb" | "g" | "gb"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        program   = "fleet.mensura"
        namespace = "fleet"

        [target]
        kind  = "compose"
        name  = "mensura-fleet"
        image = "ghcr.io/mensura-lang/mensura:0.1.0"

        [database]
        kind = "sqlite"

        [[instance]]
        name    = "mensura"
        roles   = ["intake", "read", "materializer"]
        publish = "127.0.0.1:8080"

        [[principal]]
        id           = "spiffe://local/service/dashboard"
        token_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
        scopes       = ["read:all"]
    "#;

    const INSTANCE: &str = r#"
        [[instance]]
        name    = "mensura"
        roles   = ["intake", "read", "materializer"]
        publish = "127.0.0.1:8080"
"#;

    fn err(text: &str) -> String {
        DeployConfig::parse(text).unwrap_err().to_string()
    }

    #[test]
    fn the_minimal_configuration_parses_as_written() {
        let config = DeployConfig::parse(MINIMAL).unwrap();
        assert_eq!(config.database.kind, "sqlite");
        assert_eq!(config.instances.len(), 1);
        let only = &config.instances[0];
        assert_eq!(only.name, "mensura");
        assert_eq!(only.roles, [Role::Intake, Role::Read, Role::Materializer]);
        assert_eq!(only.publish_addr().to_string(), "127.0.0.1:8080");
        assert_eq!(only.memory, None);
    }

    #[test]
    fn database_instances_roles_and_publish_have_no_defaults() {
        let text = MINIMAL.replace("[database]\n        kind = \"sqlite\"", "");
        assert!(err(&text).contains("database"));
        let text = MINIMAL.replace("kind = \"sqlite\"", "");
        assert!(err(&text).contains("kind"));
        let text = MINIMAL.replace(INSTANCE, "\n");
        assert!(err(&text).contains("instance"));
        let text = MINIMAL.replace(INSTANCE, "\n").replace(
            "namespace = \"fleet\"",
            "namespace = \"fleet\"\n        instance  = []",
        );
        assert!(err(&text).contains("at least one"));
        let text = MINIMAL.replace("roles   = [\"intake\", \"read\", \"materializer\"]", "");
        assert!(err(&text).contains("roles"));
        let text = MINIMAL.replace("publish = \"127.0.0.1:8080\"", "");
        assert!(err(&text).contains("publish"));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let text = MINIMAL.replace("namespace = \"fleet\"", "namespace = \"fleet\"\nport = 1");
        assert!(err(&text).contains("port"));
    }

    #[test]
    fn only_compose_and_sqlite_are_admitted() {
        let text = MINIMAL.replace("kind  = \"compose\"", "kind  = \"kubernetes\"");
        assert!(err(&text).contains("`compose` is the only target"));
        let text = MINIMAL.replace("kind = \"sqlite\"", "kind = \"postgres\"");
        assert!(err(&text).contains("Postgres backend"));
    }

    #[test]
    fn sqlite_admits_only_the_all_in_one_instance() {
        let second = INSTANCE
            .replace("\"mensura\"", "\"b\"")
            .replace("8080", "8081");
        let two = format!("{MINIMAL}{second}");
        assert!(err(&two).contains("all-in-one"));
        let subset = MINIMAL.replace("[\"intake\", \"read\", \"materializer\"]", "[\"read\"]");
        assert!(err(&subset).contains("1 of the three roles"));
    }

    #[test]
    fn a_deployment_needs_a_principal() {
        let text = MINIMAL.split("[[principal]]").next().unwrap();
        assert!(err(text).contains("mensura token"));
    }

    #[test]
    fn names_addresses_and_sizes_are_checked() {
        let text = MINIMAL.replace("mensura-fleet", "Mensura Fleet");
        assert!(err(&text).contains("target.name"));
        let text = MINIMAL.replace("\"fleet\"", "\"../fleet\"");
        assert!(err(&text).contains("namespace"));
        let text = MINIMAL.replace("\"127.0.0.1:8080\"", "\"8080\"");
        assert!(err(&text).contains("publish"));
        let with_memory = |m: &str| {
            MINIMAL.replace(
                "publish = \"127.0.0.1:8080\"",
                &format!("publish = \"127.0.0.1:8080\"\n        memory  = \"{m}\""),
            )
        };
        assert!(err(&with_memory("lots")).contains("memory"));
        DeployConfig::parse(&with_memory("256M")).unwrap();
    }

    #[test]
    fn byte_values() {
        for ok in ["256M", "1g", "512mb", "1048576", "64k"] {
            assert!(is_byte_value(ok), "{ok}");
        }
        for bad in ["", "M", "1.5G", "256 M", "1t"] {
            assert!(!is_byte_value(bad), "{bad}");
        }
    }
}
