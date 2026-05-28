//! Where micro keeps things.

use std::path::PathBuf;

/// The variable that names one directory for everything.
pub const MICRO_DIR_ENV: &str = "MICRO_DIR";

/// The XDG variable naming where configuration goes, when it is not the default.
pub const CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";

/// The XDG variable naming where data goes, when it is not the default.
pub const DATA_HOME_ENV: &str = "XDG_DATA_HOME";

/// What micro's own directory is called under whichever base directory holds it.
pub const DIR_NAME: &str = "micro";

/// The single directory micro used before it had two, and still uses wherever one exists.
pub const LEGACY_DIR_NAME: &str = ".micro";


pub fn config_dir() -> Option<PathBuf> {
    Places::from_env().config_dir()
}


pub fn data_dir() -> Option<PathBuf> {
    Places::from_env().data_dir()
}

/// The variables a resolution depends on, read once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    /// `MICRO_DIR`: one directory for everything.
    pub micro_dir: Option<PathBuf>,
    /// The user's home directory, which the rest are relative to.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, when it names an absolute path.
    pub config_home: Option<PathBuf>,
    /// `XDG_DATA_HOME`, when it names an absolute path.
    pub data_home: Option<PathBuf>,
}

impl Places {
    pub fn from_env() -> Places {
        Places {
            micro_dir: named(MICRO_DIR_ENV),
            home: home_dir(),
            config_home: named(CONFIG_HOME_ENV),
            data_home: named(DATA_HOME_ENV),
        }
    }

    pub fn config_dir(&self) -> Option<PathBuf> {
        self.single()
            .or_else(|| self.under(self.config_home.as_deref(), &[".config"]))
    }

    pub fn data_dir(&self) -> Option<PathBuf> {
        self.single()
            .or_else(|| self.under(self.data_home.as_deref(), &[".local", "share"]))
    }

    /// The directory that holds everything, when there is one: what `MICRO_DIR` names, or a
    /// `~/.micro` that is already there.
    fn single(&self) -> Option<PathBuf> {
        if let Some(named) = &self.micro_dir {
            return Some(named.clone());
        }
        self.legacy_dir().filter(|dir| dir.is_dir())
    }

    /// Where a pre-split install keeps everything, whether or not it is there.
    fn legacy_dir(&self) -> Option<PathBuf> {
        self.home.as_ref().map(|home| home.join(LEGACY_DIR_NAME))
    }

    
    fn under(&self, named: Option<&std::path::Path>, default: &[&str]) -> Option<PathBuf> {
        let base = match named.filter(|path| path.is_absolute()) {
            Some(named) => named.to_path_buf(),
            None => {
                let mut base = self.home.clone()?;
                base.extend(default);
                base
            }
        };
        Some(base.join(DIR_NAME))
    }
}

/// One variable as a directory, treating unset, empty and blank alike.
fn named(variable: &str) -> Option<PathBuf> {
    std::env::var(variable)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn home_dir() -> Option<PathBuf> {
    named("HOME").or_else(|| named("USERPROFILE"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A directory of its own for each test, so one that creates a legacy `.micro` cannot change
    /// what another resolves.
    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "micro-dirs-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn places(home: &Path) -> Places {
        Places {
            micro_dir: None,
            home: Some(home.to_path_buf()),
            config_home: None,
            data_home: None,
        }
    }

    #[test]
    fn a_named_directory_holds_everything() {
        let home = scratch("named");
        std::fs::create_dir_all(home.join(LEGACY_DIR_NAME)).unwrap();
        let places = Places {
            micro_dir: Some(PathBuf::from("/opt/micro")),
            config_home: Some(PathBuf::from("/opt/config")),
            data_home: Some(PathBuf::from("/opt/data")),
            ..places(&home)
        };

        assert_eq!(places.config_dir(), Some(PathBuf::from("/opt/micro")));
        assert_eq!(places.data_dir(), Some(PathBuf::from("/opt/micro")));
    }

    
    #[test]
    fn an_existing_micro_directory_keeps_holding_everything() {
        let home = scratch("legacy");
        let legacy = home.join(LEGACY_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();

        let places = places(&home);
        assert_eq!(places.config_dir(), Some(legacy.clone()));
        assert_eq!(places.data_dir(), Some(legacy));
    }

    #[test]
    fn a_fresh_install_splits_what_was_written_from_what_was_produced() {
        let home = scratch("fresh");
        let places = places(&home);

        assert_eq!(places.config_dir(), Some(home.join(".config/micro")));
        assert_eq!(places.data_dir(), Some(home.join(".local/share/micro")));
    }

    #[test]
    fn the_xdg_variables_move_a_fresh_install_off_the_defaults() {
        let home = scratch("xdg");
        let places = Places {
            config_home: Some(PathBuf::from("/srv/settings")),
            data_home: Some(PathBuf::from("/srv/state")),
            ..places(&home)
        };

        assert_eq!(places.config_dir(), Some(PathBuf::from("/srv/settings/micro")));
        assert_eq!(places.data_dir(), Some(PathBuf::from("/srv/state/micro")));
    }

    /// An existing `~/.micro` is answered for before the XDG variables are consulted.
    #[test]
    fn the_xdg_variables_do_not_move_an_install_that_already_exists() {
        let home = scratch("xdg-legacy");
        let legacy = home.join(LEGACY_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        let places = Places {
            config_home: Some(PathBuf::from("/srv/settings")),
            ..places(&home)
        };

        assert_eq!(places.config_dir(), Some(legacy));
    }

    #[test]
    fn a_relative_xdg_variable_is_read_as_if_it_were_unset() {
        let home = scratch("relative");
        let places = Places {
            config_home: Some(PathBuf::from("settings")),
            ..places(&home)
        };

        assert_eq!(places.config_dir(), Some(home.join(".config/micro")));
    }

    #[test]
    fn a_file_named_micro_in_the_home_directory_is_not_an_install() {
        let home = scratch("file");
        std::fs::write(home.join(LEGACY_DIR_NAME), "not a directory").unwrap();

        assert_eq!(places(&home).config_dir(), Some(home.join(".config/micro")));
    }

    #[test]
    fn nothing_resolves_without_a_home_or_a_variable_naming_one() {
        let places = Places::default();
        assert_eq!(places.config_dir(), None);
        assert_eq!(places.data_dir(), None);

        let named = Places {
            data_home: Some(PathBuf::from("/srv/state")),
            ..Places::default()
        };
        assert_eq!(named.data_dir(), Some(PathBuf::from("/srv/state/micro")));
        assert_eq!(named.config_dir(), None);
    }
}
