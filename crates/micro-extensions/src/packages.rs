//! Installing extensions that live somewhere else.
//!
//! A source names where a package comes from, in the forms ohm accepts:
//!
//! ```text
//! npm:@scope/name          npm:name@1.2.3
//! git:github.com/user/repo git:git@github.com:user/repo
//! https://github.com/user/repo   ssh://git@github.com/user/repo
//! ./local/path
//! ```
//!
//! What is fetched lands under micro's own directory — `npm/node_modules/<name>` or
//! `git/<host>/<owner>/<repo>` — and the source is written into the settings, so the next
//! run finds it without being told again. Nothing is installed into the workspace, and
//! nothing is installed globally.

use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

/// Where a package comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A package on the npm registry.
    Npm {
        /// The whole spec as written, which is what the installer is given.
        spec: String,
        /// The package name, without a version.
        name: String,
        /// The version or range, when one was asked for.
        version: Option<String>,
    },
    /// A repository to clone.
    Git {
        /// The URL to clone from.
        url: String,
        /// Where it goes, as `host/owner/repo`.
        slug: String,
        /// A branch, tag or commit to check out.
        reference: Option<String>,
    },
    /// A directory already on this machine.
    Local { path: String },
}

impl Source {
    /// Read a source the way ohm reads it.
    pub fn parse(source: &str) -> Result<Source, String> {
        let source = source.trim();
        if source.is_empty() {
            return Err("say what to install".to_string());
        }

        if let Some(spec) = source.strip_prefix("npm:") {
            return npm(spec.trim());
        }
        if is_local(source) {
            return Ok(Source::Local {
                path: source.to_string(),
            });
        }
        if let Some(parsed) = git(source) {
            return Ok(parsed);
        }
        // Anything left is taken as a path, which is what ohm does: a bare name is more
        // likely a directory than a URL somebody forgot the scheme for.
        Ok(Source::Local {
            path: source.to_string(),
        })
    }

    /// How the source is written back into the settings.
    pub fn canonical(&self) -> String {
        match self {
            Source::Npm { name, version, .. } => match version {
                Some(version) => format!("npm:{name}@{version}"),
                None => format!("npm:{name}"),
            },
            Source::Git { url, reference, .. } => match reference {
                Some(reference) => format!("{url}#{reference}"),
                None => url.clone(),
            },
            Source::Local { path } => path.clone(),
        }
    }

    /// Where this source is installed, under `home` for a user install or under the
    /// workspace for a project one.
    pub fn install_path(&self, home: &Path, workspace: &Path, local: bool) -> PathBuf {
        let base = match local {
            true => workspace.join(".micro"),
            false => home.to_path_buf(),
        };
        match self {
            Source::Npm { name, .. } => base.join("npm").join("node_modules").join(name),
            Source::Git { slug, .. } => base.join("git").join(slug),
            Source::Local { path } => match Path::new(path).is_absolute() {
                true => PathBuf::from(path),
                false => workspace.join(path),
            },
        }
    }
}

/// `@scope/name@1.2.3` split into what it names and what it pins.
fn npm(spec: &str) -> Result<Source, String> {
    if spec.is_empty() {
        return Err("say which npm package to install".to_string());
    }

    // A scope starts with `@`, and its own slash is not a version separator.
    let after_scope = match spec.starts_with('@') {
        true => spec.find('/').map(|slash| slash + 1).unwrap_or(0),
        false => 0,
    };
    let (name, version) = match spec[after_scope..].find('@') {
        Some(at) => {
            let at = after_scope + at;
            (&spec[..at], Some(spec[at + 1..].to_string()))
        }
        None => (spec, None),
    };

    if name.is_empty() {
        return Err(format!("`{spec}` does not name a package"));
    }
    Ok(Source::Npm {
        spec: spec.to_string(),
        name: name.to_string(),
        version: version.filter(|version| !version.is_empty()),
    })
}

/// A repository, in any of the spellings ohm accepts.
fn git(source: &str) -> Option<Source> {
    let (rest, reference) = match source.split_once('#') {
        Some((rest, reference)) if !reference.is_empty() => (rest, Some(reference.to_string())),
        _ => (source, None),
    };

    let stripped = rest.strip_prefix("git:").unwrap_or(rest);
    let (url, slug) = if let Some(rest) = stripped.strip_prefix("https://") {
        (format!("https://{rest}"), slug_of(rest))
    } else if let Some(rest) = stripped.strip_prefix("http://") {
        (format!("http://{rest}"), slug_of(rest))
    } else if let Some(rest) = stripped.strip_prefix("ssh://") {
        (format!("ssh://{rest}"), slug_of(strip_user(rest)))
    } else if stripped.contains('@') && stripped.contains(':') {
        // `git@github.com:user/repo`
        let after_user = stripped.split_once('@').map(|(_, rest)| rest)?;
        let (host, path) = after_user.split_once(':')?;
        (stripped.to_string(), slug_of(&format!("{host}/{path}")))
    } else if stripped.contains('/') && stripped.contains('.') {
        // `github.com/user/repo`
        (format!("https://{stripped}"), slug_of(stripped))
    } else {
        return None;
    };

    Some(Source::Git {
        url,
        slug,
        reference,
    })
}

/// `github.com/user/repo.git` as the directory it is cloned into.
fn slug_of(path: &str) -> String {
    path.trim_end_matches('/')
        .trim_end_matches(".git")
        .replace(':', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn strip_user(rest: &str) -> &str {
    rest.split_once('@').map(|(_, rest)| rest).unwrap_or(rest)
}

/// Whether a source names a place on this machine rather than one to fetch from.
fn is_local(source: &str) -> bool {
    source.starts_with("./")
        || source.starts_with("../")
        || source.starts_with('/')
        || source.starts_with("~/")
        || source.starts_with("file://")
}

/// What an install produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Installed {
    /// The source, as it is written into the settings.
    pub source: String,
    /// Where it ended up.
    pub path: PathBuf,
}

/// Fetch a source and put it where micro will find it.
///
/// npm packages are installed with Bun, which is already required to run an extension.
/// A repository is cloned, or brought up to date when it is already there.
pub async fn install(
    source: &Source,
    home: &Path,
    workspace: &Path,
    local: bool,
) -> Result<Installed, String> {
    let path = source.install_path(home, workspace, local);

    match source {
        Source::Npm { spec, .. } => {
            let root = path
                .parent()
                .and_then(|modules| modules.parent())
                .ok_or("the install path has no root")?
                .to_path_buf();
            std::fs::create_dir_all(&root)
                .map_err(|error| format!("cannot use {}: {error}", root.display()))?;
            // A package.json has to exist or Bun installs into the nearest parent that
            // has one, which would put someone else's dependency in micro's home.
            let manifest = root.join("package.json");
            if !manifest.exists() {
                std::fs::write(&manifest, "{\n  \"name\": \"micro-extensions\"\n}\n")
                    .map_err(|error| format!("cannot write {}: {error}", manifest.display()))?;
            }

            let runtime = crate::host::which_bun()
                .ok_or("bun is not on the path. Install it from https://bun.sh")?;
            run(&runtime, &["add", spec.as_str()], &root).await?;
        }
        Source::Git { url, reference, .. } => {
            if path.join(".git").exists() {
                run(Path::new("git"), &["fetch", "--all", "--tags"], &path).await?;
                if let Some(reference) = reference {
                    run(Path::new("git"), &["checkout", reference], &path).await?;
                }
                run(Path::new("git"), &["pull", "--ff-only"], &path).await?;
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("cannot use {}: {error}", parent.display()))?;
                }
                let target = path.display().to_string();
                run(
                    Path::new("git"),
                    &["clone", "--depth", "1", url.as_str(), target.as_str()],
                    workspace,
                )
                .await?;
                if let Some(reference) = reference {
                    run(Path::new("git"), &["fetch", "origin", reference], &path).await?;
                    run(Path::new("git"), &["checkout", reference], &path).await?;
                }
            }
        }
        Source::Local { .. } => {
            if !path.exists() {
                return Err(format!("{} is not there", path.display()));
            }
        }
    }

    if !path.exists() {
        return Err(format!(
            "the install finished but {} is not there",
            path.display()
        ));
    }

    Ok(Installed {
        source: source.canonical(),
        path,
    })
}

/// Take an installed package away. Its source is the caller's to forget.
pub fn remove(source: &Source, home: &Path, workspace: &Path, local: bool) -> Result<(), String> {
    let path = source.install_path(home, workspace, local);
    match source {
        // A local source was never copied anywhere, so there is nothing to delete.
        Source::Local { .. } => Ok(()),
        _ => match path.exists() {
            false => Ok(()),
            true => std::fs::remove_dir_all(&path)
                .map_err(|error| format!("cannot remove {}: {error}", path.display())),
        },
    }
}

/// Run a program with its arguments, and say what it printed when it fails.
///
/// No shell is involved: the arguments go to the program as they are written, so a package
/// name carrying shell punctuation is a package name and nothing else.
async fn run(program: &Path, arguments: &[&str], directory: &Path) -> Result<String, String> {
    let finished = tokio::process::Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .map_err(|error| {
            format!(
                "cannot run {}: {error}",
                program.file_name().unwrap_or_default().to_string_lossy()
            )
        })?;

    let stdout = String::from_utf8_lossy(&finished.stdout).into_owned();
    if finished.status.success() {
        return Ok(stdout);
    }

    let stderr = String::from_utf8_lossy(&finished.stderr);
    let said = match stderr.trim().is_empty() {
        true => stdout.trim().to_string(),
        false => stderr.trim().to_string(),
    };
    Err(format!(
        "{} failed: {said}",
        program.file_name().unwrap_or_default().to_string_lossy()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_npm_package_is_read_with_its_version() {
        let Source::Npm {
            spec,
            name,
            version,
        } = Source::parse("npm:micro-mcp-adapter").unwrap()
        else {
            panic!("an npm package");
        };
        assert_eq!(spec, "micro-mcp-adapter");
        assert_eq!(name, "micro-mcp-adapter");
        assert_eq!(version, None);

        let Source::Npm { name, version, .. } = Source::parse("npm:thing@1.2.3").unwrap() else {
            panic!("an npm package");
        };
        assert_eq!(name, "thing");
        assert_eq!(version.as_deref(), Some("1.2.3"));
    }

    /// A scope's own `@` is part of the name, not the start of a version.
    #[test]
    fn a_scoped_package_keeps_its_scope() {
        let Source::Npm { name, version, .. } = Source::parse("npm:@foo/bar").unwrap() else {
            panic!("an npm package");
        };
        assert_eq!(name, "@foo/bar");
        assert_eq!(version, None);

        let Source::Npm { name, version, .. } = Source::parse("npm:@foo/bar@^2.0.0").unwrap()
        else {
            panic!("an npm package");
        };
        assert_eq!(name, "@foo/bar");
        assert_eq!(version.as_deref(), Some("^2.0.0"));
    }

    #[test]
    fn every_spelling_of_a_repository_is_understood() {
        for source in [
            "git:github.com/user/repo",
            "https://github.com/user/repo",
            "https://github.com/user/repo.git",
            "git:https://github.com/user/repo",
            "ssh://git@github.com/user/repo",
            "git:git@github.com:user/repo",
        ] {
            let Source::Git { slug, .. } = Source::parse(source).unwrap() else {
                panic!("{source} is a repository");
            };
            assert_eq!(slug, "github.com/user/repo", "{source}");
        }
    }

    #[test]
    fn a_reference_can_be_asked_for() {
        let Source::Git { url, reference, .. } =
            Source::parse("https://github.com/user/repo#v2").unwrap()
        else {
            panic!("a repository");
        };
        assert_eq!(url, "https://github.com/user/repo");
        assert_eq!(reference.as_deref(), Some("v2"));
    }

    #[test]
    fn a_path_is_taken_as_a_path() {
        for source in ["./local/thing", "../beside", "/absolute/thing", "plainname"] {
            let Source::Local { path } = Source::parse(source).unwrap() else {
                panic!("{source} is a path");
            };
            assert_eq!(path, source);
        }
    }

    #[test]
    fn nothing_is_not_a_source() {
        assert!(Source::parse("   ").is_err());
        assert!(Source::parse("npm:").is_err());
    }

    /// What is fetched goes under micro's own directory, or under the project when the
    /// install is a project one. Never into the workspace root, never global.
    #[test]
    fn an_install_lands_where_it_is_looked_for() {
        let home = Path::new("/home/.micro");
        let workspace = Path::new("/work");

        let npm = Source::parse("npm:@foo/bar").unwrap();
        assert_eq!(
            npm.install_path(home, workspace, false),
            PathBuf::from("/home/.micro/npm/node_modules/@foo/bar")
        );
        assert_eq!(
            npm.install_path(home, workspace, true),
            PathBuf::from("/work/.micro/npm/node_modules/@foo/bar")
        );

        let repository = Source::parse("https://github.com/user/repo").unwrap();
        assert_eq!(
            repository.install_path(home, workspace, false),
            PathBuf::from("/home/.micro/git/github.com/user/repo")
        );

        let relative = Source::parse("./thing").unwrap();
        assert_eq!(
            relative.install_path(home, workspace, false),
            PathBuf::from("/work/./thing")
        );
    }

    /// A source is written back the way it will be read again.
    #[test]
    fn a_source_survives_being_written_down() {
        for source in [
            "npm:@foo/bar",
            "npm:thing@1.2.3",
            "https://github.com/user/repo",
            "./local/thing",
        ] {
            let parsed = Source::parse(source).unwrap();
            let written = parsed.canonical();
            assert_eq!(Source::parse(&written).unwrap(), parsed, "{source}");
        }
    }

    #[tokio::test]
    async fn a_local_source_that_is_not_there_is_refused() {
        let error = install(
            &Source::parse("/nowhere-at-all").unwrap(),
            Path::new("/tmp"),
            Path::new("/tmp"),
            false,
        )
        .await
        .expect_err("nothing is there");
        assert!(error.contains("is not there"), "{error}");
    }

    #[test]
    fn removing_a_local_source_leaves_it_where_it_is() {
        let root = std::env::temp_dir().join(format!("micro-packages-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = Source::parse(&root.display().to_string()).unwrap();

        remove(&source, Path::new("/tmp"), Path::new("/tmp"), false).unwrap();
        assert!(root.exists(), "a local source is not micro's to delete");

        let _ = std::fs::remove_dir_all(&root);
    }
}
