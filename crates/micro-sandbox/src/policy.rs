//! What a session may touch, expressed as a policy the OS sandbox and the file tools both read.

use serde::de::Error as _;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

/// The directory names that stay read-only at the top of every writable root.
pub const PROTECTED_NAMES: [&str; 2] = [".git", ".micro"];

const READ_ONLY: &str = "read-only";
const WORKSPACE_WRITE: &str = "workspace-write";
const FULL: &str = "full";

/// What commands run under this session are allowed to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxPolicy {
    /// Read anything, write nothing, no network.
    ReadOnly,

    /// Read anything; write inside the workspace and whatever else was granted explicitly, minus
    /// the protected paths inside those roots.
    WorkspaceWrite {
        /// Directories beyond the workspace that may be written to.
        writable_roots: Vec<PathBuf>,

        /// Whether the command may reach the network.
        allow_network: bool,
    },

    /// No confinement at all.
    Full,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        SandboxPolicy::workspace_write()
    }
}

impl SandboxPolicy {
    pub fn workspace_write() -> Self {
        SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            allow_network: false,
        }
    }

    /// The name this policy is written as in config, on the command line, and in the ledger.
    pub fn name(&self) -> &'static str {
        match self {
            SandboxPolicy::ReadOnly => READ_ONLY,
            SandboxPolicy::WorkspaceWrite { .. } => WORKSPACE_WRITE,
            SandboxPolicy::Full => FULL,
        }
    }

    /// Whether the policy leaves writes unrestricted.
    pub fn allows_all_writes(&self) -> bool {
        matches!(self, SandboxPolicy::Full)
    }

    /// Whether the policy lets a command reach the network.
    pub fn allows_network(&self) -> bool {
        match self {
            SandboxPolicy::ReadOnly => false,
            SandboxPolicy::WorkspaceWrite { allow_network, .. } => *allow_network,
            SandboxPolicy::Full => true,
        }
    }

    /// The extra roots this policy grants beyond the workspace.
    pub fn granted_roots(&self) -> &[PathBuf] {
        match self {
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } => writable_roots,
            SandboxPolicy::ReadOnly | SandboxPolicy::Full => &[],
        }
    }

    /// Add network access to a workspace-write policy.
    pub fn with_network(mut self) -> Self {
        if let SandboxPolicy::WorkspaceWrite { allow_network, .. } = &mut self {
            *allow_network = true;
        }
        self
    }

    /// Add one writable root to a workspace-write policy.
    pub fn with_writable_root(mut self, root: impl Into<PathBuf>) -> Self {
        if let SandboxPolicy::WorkspaceWrite { writable_roots, .. } = &mut self {
            let root = root.into();
            if !writable_roots.contains(&root) {
                writable_roots.push(root);
            }
        }
        self
    }
}

impl fmt::Display for SandboxPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A directory that may be written to, along with the paths inside it that may not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritableRoot {
    pub root: PathBuf,
    pub read_only_subpaths: Vec<PathBuf>,
}

impl WritableRoot {
    /// Whether `path` may be written to.
    pub fn is_path_writable(&self, path: &Path) -> bool {
        if !path.starts_with(&self.root) {
            return false;
        }
        !self
            .read_only_subpaths
            .iter()
            .any(|subpath| path.starts_with(subpath))
    }

    /// The read-only subpath `path` falls inside, if any.
    pub fn protecting(&self, path: &Path) -> Option<&Path> {
        self.read_only_subpaths
            .iter()
            .find(|subpath| path.starts_with(subpath))
            .map(PathBuf::as_path)
    }
}

/// A policy name that is not one of the three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPolicy {
    name: String,
}

impl fmt::Display for UnknownPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown sandbox policy {:?}, expected one of {READ_ONLY}, {WORKSPACE_WRITE}, {FULL}",
            self.name
        )
    }
}

impl std::error::Error for UnknownPolicy {}

impl FromStr for SandboxPolicy {
    type Err = UnknownPolicy;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            READ_ONLY => Ok(SandboxPolicy::ReadOnly),
            WORKSPACE_WRITE => Ok(SandboxPolicy::workspace_write()),
            FULL => Ok(SandboxPolicy::Full),
            other => Err(UnknownPolicy {
                name: other.to_string(),
            }),
        }
    }
}

/// The two shapes a policy is written in: the bare name, or a table that spells out what
/// `workspace-write` grants beyond the default.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum PolicyRepr {
    Name(String),
    Detailed(DetailedPolicy),
}

#[derive(Serialize, Deserialize)]
struct DetailedPolicy {
    mode: String,
    #[serde(default)]
    writable_roots: Vec<PathBuf>,
    #[serde(default)]
    allow_network: bool,
}

impl Serialize for SandboxPolicy {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                allow_network,
            } if !writable_roots.is_empty() || *allow_network => DetailedPolicy {
                mode: WORKSPACE_WRITE.to_string(),
                writable_roots: writable_roots.clone(),
                allow_network: *allow_network,
            }
            .serialize(serializer),
            plain => serializer.serialize_str(plain.name()),
        }
    }
}

impl<'de> Deserialize<'de> for SandboxPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match PolicyRepr::deserialize(deserializer)? {
            PolicyRepr::Name(name) => name.parse().map_err(D::Error::custom),
            PolicyRepr::Detailed(detailed) => match detailed.mode.as_str() {
                READ_ONLY => Ok(SandboxPolicy::ReadOnly),
                WORKSPACE_WRITE => Ok(SandboxPolicy::WorkspaceWrite {
                    writable_roots: detailed.writable_roots,
                    allow_network: detailed.allow_network,
                }),
                FULL => Ok(SandboxPolicy::Full),
                other => Err(D::Error::custom(UnknownPolicy {
                    name: other.to_string(),
                })),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_policy_name_deserializes_with_the_defaults_filled_in() {
        let policy: SandboxPolicy = serde_json::from_str("\"workspace-write\"").unwrap();
        assert_eq!(policy, SandboxPolicy::workspace_write());
        assert_eq!(
            serde_json::from_str::<SandboxPolicy>("\"read-only\"").unwrap(),
            SandboxPolicy::ReadOnly
        );
        assert_eq!(
            serde_json::from_str::<SandboxPolicy>("\"full\"").unwrap(),
            SandboxPolicy::Full
        );
    }

    #[test]
    fn a_spelled_out_workspace_write_keeps_its_grants() {
        let policy: SandboxPolicy = serde_json::from_str(
            r#"{"mode":"workspace-write","writable_roots":["/srv/cache"],"allow_network":true}"#,
        )
        .unwrap();
        assert_eq!(
            policy,
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![PathBuf::from("/srv/cache")],
                allow_network: true,
            }
        );
    }

    #[test]
    fn a_policy_with_nothing_extra_to_say_serializes_as_its_name() {
        let rendered = serde_json::to_string(&SandboxPolicy::workspace_write()).unwrap();
        assert_eq!(rendered, "\"workspace-write\"");
    }

    #[test]
    fn a_policy_with_grants_survives_a_round_trip() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![PathBuf::from("/srv/cache")],
            allow_network: true,
        };
        let rendered = serde_json::to_string(&policy).unwrap();
        assert_eq!(
            serde_json::from_str::<SandboxPolicy>(&rendered).unwrap(),
            policy
        );
    }

    #[test]
    fn an_unknown_policy_name_is_refused_by_name() {
        let error = "yolo".parse::<SandboxPolicy>().unwrap_err().to_string();
        assert!(error.contains("yolo"), "{error}");
        assert!(error.contains("workspace-write"), "{error}");
    }

    #[test]
    fn a_writable_root_keeps_its_protected_subpaths_read_only() {
        let root = WritableRoot {
            root: PathBuf::from("/work"),
            read_only_subpaths: vec![PathBuf::from("/work/.git")],
        };
        assert!(root.is_path_writable(Path::new("/work/src/main.rs")));
        assert!(!root.is_path_writable(Path::new("/work/.git")));
        assert!(!root.is_path_writable(Path::new("/work/.git/hooks/pre-commit")));
        assert!(!root.is_path_writable(Path::new("/elsewhere/file")));
    }
}
