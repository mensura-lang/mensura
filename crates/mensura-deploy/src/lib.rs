//! `mensura deploy`: compile a deploy configuration and a program into a
//! bundle (`docs/decisions/0046-deploy-configuration-and-bundle.md`,
//! `docs/toolkit/07-deploying.md`).
//!
//! Deploy is a pure function of its inputs.  [`compile`] reads the deploy
//! configuration and the program, validates both, and renders every file
//! of the bundle in memory, parsing each runtime configuration with
//! `serve`'s own loader so that deploy can never emit a file `serve` would
//! refuse.  [`Bundle::write`] then writes the files it owns and removes
//! nothing.  Nothing secret is generated or written.

pub mod compose;
pub mod config;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub use config::DeployConfig;

/// Where the runtime configurations and the program are mounted inside
/// every container.
pub const CONFIG_DIR: &str = "/etc/mensura";
/// Where the data volume is mounted inside every container.
pub const DATA_DIR: &str = "/var/lib/mensura";
/// The port every instance listens on inside its container.
pub const CONTAINER_PORT: u16 = 8080;
/// The environment variable the runtime configuration's database
/// reference names.
pub const DATABASE_ENV: &str = "MENSURA_DATABASE_URL";

/// Why a bundle was not produced.
#[derive(Debug)]
pub enum DeployError {
    Io(PathBuf, std::io::Error),
    /// The deploy configuration does not parse or validate.
    Invalid(String),
    /// The program has diagnostics, already formatted one per entry.
    Program(Vec<String>),
}

impl fmt::Display for DeployError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeployError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            DeployError::Invalid(msg) => write!(f, "{msg}"),
            DeployError::Program(diagnostics) => write!(f, "{}", diagnostics.join("\n")),
        }
    }
}

impl std::error::Error for DeployError {}

/// One file of a bundle, at a path relative to the bundle's root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleFile {
    pub path: PathBuf,
    pub contents: String,
    pub executable: bool,
}

/// A rendered bundle: its files and the notes deploy prints.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub files: Vec<BundleFile>,
    /// Advice that does not refuse the deployment, such as an instance
    /// with no memory limit.
    pub notes: Vec<String>,
}

impl Bundle {
    /// The file at `path`, if the bundle has one.
    pub fn file(&self, path: &str) -> Option<&BundleFile> {
        self.files.iter().find(|f| f.path == Path::new(path))
    }

    /// Write every file under `out`, creating directories as needed.
    /// Files the bundle does not own are left alone.
    pub fn write(&self, out: &Path) -> Result<(), DeployError> {
        for file in &self.files {
            let path = out.join(&file.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| DeployError::Io(parent.to_path_buf(), e))?;
            }
            std::fs::write(&path, &file.contents).map_err(|e| DeployError::Io(path.clone(), e))?;
            if file.executable {
                make_executable(&path)?;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), DeployError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| DeployError::Io(path.to_path_buf(), e))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), DeployError> {
    Ok(())
}

/// The default output directory: `bundle` beside the configuration file.
pub fn default_out(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("bundle")
}

/// Compile the deploy configuration at `config_path` into a bundle.
pub fn compile(config_path: &Path) -> Result<Bundle, DeployError> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|e| DeployError::Io(config_path.to_path_buf(), e))?;
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let name = config_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "deploy.toml".to_string());
    compile_text(&text, base, &name).map_err(|e| match e {
        DeployError::Invalid(msg) => DeployError::Invalid(format!("{name}: {msg}")),
        other => other,
    })
}

/// Compile configuration text, resolving the program against `base`.
/// `config_name` is the file name the generated files cite as their
/// source; no host path enters the bundle.
pub fn compile_text(text: &str, base: &Path, config_name: &str) -> Result<Bundle, DeployError> {
    let config = DeployConfig::parse(text)?;
    let program_path = base.join(&config.program);
    let source = std::fs::read_to_string(&program_path)
        .map_err(|e| DeployError::Io(program_path.clone(), e))?;
    mensura_serve::frontend::resolve_source(&program_path, &source)
        .map_err(DeployError::Program)?;
    let program_name = config
        .program
        .file_name()
        .expect("validated at parse")
        .to_string_lossy()
        .into_owned();

    let mut files = vec![BundleFile {
        path: Path::new("mensura").join(&program_name),
        contents: source,
        executable: false,
    }];
    let mut notes = Vec::new();
    let mut digests = BTreeMap::new();
    for instance in &config.instances {
        let runtime = compose::runtime_config(&config, instance, &program_name, config_name);
        // Parse what is about to be written with `serve`'s own loader.
        mensura_serve::Config::parse(&runtime, Path::new(CONFIG_DIR)).map_err(|e| {
            DeployError::Invalid(format!(
                "the runtime configuration of instance `{}` would not load in \
                 `serve`: {e}",
                instance.name
            ))
        })?;
        // A container's mounted files are invisible to compose, which
        // recreates a service only when its definition changes; the
        // digest of what the instance reads puts them in the definition,
        // so that a regenerated bundle recreates exactly the instances
        // whose files changed (decision 7).
        digests.insert(
            instance.name.clone(),
            mounted_digest(&[&files[0].contents, &runtime]),
        );
        files.push(BundleFile {
            path: Path::new("mensura").join(format!("{}.toml", instance.name)),
            contents: runtime,
            executable: false,
        });
        if instance.memory.is_none() {
            notes.push(format!(
                "instance `{}` has no memory limit; on a shared host, set `memory` \
                 so that memory pressure stops it before its neighbours",
                instance.name
            ));
        }
    }
    files.push(BundleFile {
        path: PathBuf::from("compose.yaml"),
        contents: compose::compose_file(&config, config_name, &digests),
        executable: false,
    });
    files.push(BundleFile {
        path: PathBuf::from("up.sh"),
        contents: compose::up_script(),
        executable: true,
    });
    files.push(BundleFile {
        path: PathBuf::from("down.sh"),
        contents: compose::down_script(),
        executable: true,
    });
    Ok(Bundle { files, notes })
}

/// The SHA-256 of the files an instance mounts, each length-prefixed so
/// that no two different sets of contents collide by concatenation.
fn mounted_digest(contents: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for c in contents {
        hasher.update((c.len() as u64).to_be_bytes());
        hasher.update(c.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn examples() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples")
    }

    const FLEET: &str = r#"
        program   = "fleet-monitoring.mensura"
        namespace = "fleet"

        [target]
        kind  = "compose"
        name  = "mensura-fleet"
        image = "ghcr.io/mensura-lang/mensura:sha-1a2b3c4"

        [[instance]]
        name    = "mensura"
        publish = "127.0.0.1:8080"
        memory  = "256M"

        [[principal]]
        id           = "spiffe://plant.acme/service/dashboard"
        token_sha256 = "60303ae22b998861bce3b28f33eec1be758a213c86c93c076dbe9f558c11c752"
        scopes       = ["read:all"]
    "#;

    #[test]
    fn the_fleet_bundle_has_every_file() {
        let bundle = compile_text(FLEET, &examples(), "deploy.toml").unwrap();
        let paths: Vec<_> = bundle
            .files
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            paths,
            [
                "mensura/fleet-monitoring.mensura",
                "mensura/mensura.toml",
                "compose.yaml",
                "up.sh",
                "down.sh",
            ]
        );
        assert!(bundle.notes.is_empty(), "{:?}", bundle.notes);
        assert!(bundle.file("up.sh").unwrap().executable);
    }

    #[test]
    fn the_runtime_configuration_loads_in_serve() {
        let bundle = compile_text(FLEET, &examples(), "deploy.toml").unwrap();
        let text = &bundle.file("mensura/mensura.toml").unwrap().contents;
        let config = mensura_serve::Config::parse(text, Path::new(CONFIG_DIR)).unwrap();
        assert_eq!(
            config.program,
            Path::new("/etc/mensura/fleet-monitoring.mensura")
        );
        assert_eq!(config.namespace, "fleet");
        assert_eq!(config.listen, "0.0.0.0:8080");
        assert_eq!(config.principals.len(), 1);
        assert_eq!(
            config.database,
            mensura_serve::config::SecretRef::Env {
                env: DATABASE_ENV.to_string()
            }
        );
    }

    #[test]
    fn the_compose_file_wires_volume_limit_port_and_health() {
        let bundle = compile_text(FLEET, &examples(), "deploy.toml").unwrap();
        let compose = &bundle.file("compose.yaml").unwrap().contents;
        for needle in [
            "name: \"mensura-fleet\"",
            "image: \"ghcr.io/mensura-lang/mensura:sha-1a2b3c4\"",
            "command: [\"serve\", \"--config\", \"/etc/mensura/mensura.toml\"]",
            "MENSURA_DATABASE_URL: \"sqlite:/var/lib/mensura/fleet.db\"",
            "\"127.0.0.1:8080:8080\"",
            "memory: \"256M\"",
            "test: [\"CMD\", \"mensura\", \"health\", \"127.0.0.1:8080\"]",
            "read_only: true",
            "source: data",
        ] {
            assert!(compose.contains(needle), "missing {needle}:\n{compose}");
        }
    }

    #[test]
    fn a_changed_program_changes_the_service_definition() {
        let compose_over = |dir: &Path| {
            let bundle = compile_text(FLEET, dir, "deploy.toml").unwrap();
            bundle.file("compose.yaml").unwrap().contents.clone()
        };
        let original = compose_over(&examples());
        assert!(original.contains(compose::DIGEST_LABEL), "{original}");
        assert_eq!(original, compose_over(&examples()), "deploy is pure");

        let dir = std::env::temp_dir().join(format!("mensura-deploy-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = std::fs::read_to_string(examples().join("fleet-monitoring.mensura")).unwrap();
        std::fs::write(
            dir.join("fleet-monitoring.mensura"),
            format!("{source}\n// an edit\n"),
        )
        .unwrap();
        let edited = compose_over(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_ne!(original, edited);
    }

    #[test]
    fn an_instance_without_a_memory_limit_gets_a_note() {
        let text = FLEET.replace("memory  = \"256M\"", "");
        let bundle = compile_text(&text, &examples(), "deploy.toml").unwrap();
        assert_eq!(bundle.notes.len(), 1);
        assert!(
            !bundle
                .file("compose.yaml")
                .unwrap()
                .contents
                .contains("memory")
        );
    }

    #[test]
    fn a_program_with_diagnostics_is_refused() {
        let text = FLEET.replace("fleet-monitoring.mensura", "missing.mensura");
        assert!(matches!(
            compile_text(&text, &examples(), "deploy.toml"),
            Err(DeployError::Io(..))
        ));
        let dir = std::env::temp_dir().join(format!("mensura-deploy-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fleet-monitoring.mensura"), "store {").unwrap();
        let err = compile_text(FLEET, &dir, "deploy.toml").unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(err, DeployError::Program(_)), "{err}");
    }

    #[test]
    fn writing_keeps_files_it_does_not_own() {
        let bundle = compile_text(FLEET, &examples(), "deploy.toml").unwrap();
        let out = std::env::temp_dir().join(format!("mensura-deploy-out-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(".env"), "KEEP=1\n").unwrap();
        bundle.write(&out).unwrap();
        bundle.write(&out).unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join(".env")).unwrap(),
            "KEEP=1\n"
        );
        assert!(out.join("mensura/mensura.toml").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(out.join("up.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        std::fs::remove_dir_all(&out).unwrap();
    }
}
