//! Pull configuration from ENV vars

use std::{
    ffi::{OsStr, OsString},
    io::Write,
    num::ParseIntError,
    sync::OnceLock,
};

static CONFIG_CACHE: OnceLock<EnvConfig> = OnceLock::new();

/// Returns a cached value of global configuration.
pub(crate) fn get_config() -> &'static EnvConfig {
    CONFIG_CACHE.get_or_init(|| EnvConfig::from(std::env::vars_os(), &mut std::io::stderr()))
}

/// Global configuration
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvConfig {
    /// Maximum **additional** number of entries to show in a directory.
    ///
    /// In the input path `/path/to/dir/file.txt`, if `/path/to/dir` holds 100 entries:
    ///
    /// - When the limit is `0`, if `file.txt` exists, it will show `1` entry, or `0` if it does not.
    /// - When the limit is `10`, if `file.txt` exists, it will show `11` entries, or `10` if it does not.
    /// - If the limit is set to a number greater than `100`, it will show all entries in the directory.
    pub(crate) dir_limit: usize,
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self { dir_limit: 20 }
    }
}

impl EnvConfig {
    fn from(vars: impl IntoIterator<Item = (OsString, OsString)>, io: &mut impl Write) -> Self {
        let mut config = Self::default();
        for (name, value) in vars {
            if !name
                .to_string_lossy()
                .to_lowercase()
                .starts_with("path_facts")
            {
                continue;
            }

            match EnvKey::from_name(&name) {
                Some(key @ EnvKey::Limit) => match parse_limit(&value) {
                    Ok(limit) => {
                        config.dir_limit = limit;
                    }
                    Err(error) => print_eat_failure(
                        io,
                        format!(
                            "Invalid PATH_FACTS value for `{k}={v}`. {hint}. {error}",
                            k = key.name(),
                            v = value.to_string_lossy(),
                            hint = key.hint()
                        ),
                    ),
                },
                None => {
                    print_eat_failure(
                        io,
                        format!(
                            "Unknown PATH_FACTS env key `{}`. Must be one of: {}",
                            name.to_string_lossy(),
                            EnvKey::ALL
                                .iter()
                                .map(|key| format!("`{}`", key.name()))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                }
            }
        }
        config
    }
}

/// A configuration key from the ENV
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvKey {
    Limit,
}

impl EnvKey {
    const ALL: [Self; 1] = [Self::Limit];

    fn name(self) -> &'static str {
        match self {
            EnvKey::Limit => "PATH_FACTS_LIMIT",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            EnvKey::Limit => {
                "Must be an integer `0` or larger. Use `MAX` for the largest possible value"
            }
        }
    }

    fn from_name(name: &OsStr) -> Option<Self> {
        Self::ALL.iter().copied().find(|key| name == key.name())
    }
}

/// Silently handle stderr writes failing
fn print_eat_failure(io: &mut impl Write, msg: impl AsRef<str>) {
    let result = writeln!(io, "{}", msg.as_ref());
    if cfg!(test) {
        result.unwrap();
    }
}

fn parse_limit(value: &OsStr) -> Result<usize, String> {
    match value.to_str() {
        None => Err(format!(
            "Value is not valid UTF-8: `{}`",
            value.to_string_lossy()
        )),
        Some("MAX" | "max") => Ok(usize::MAX),
        Some(value) => value
            .parse()
            .map_err(|error: ParseIntError| error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> OsString {
        OsString::from(value)
    }

    #[test]
    fn empty_string() {
        let mut io = Vec::<u8>::new();
        let config = EnvConfig::from([(os("PATH_FACTS_LIMIT"), os(""))], &mut io);
        assert_eq!(config.dir_limit, EnvConfig::default().dir_limit);

        let warnings = String::from_utf8_lossy(&io).into_owned();
        assert!(
            warnings.contains("Invalid PATH_FACTS value for `PATH_FACTS_LIMIT=`"),
            "{}",
            warnings
        );
        assert!(warnings.contains(EnvKey::Limit.hint()), "{}", warnings);
    }

    #[test]
    fn not_an_integer() {
        let mut io = Vec::<u8>::new();
        let config = EnvConfig::from([(os("PATH_FACTS_LIMIT"), os("invalid"))], &mut io);
        assert_eq!(config.dir_limit, EnvConfig::default().dir_limit);

        let warnings = String::from_utf8_lossy(&io).into_owned();
        assert!(
            warnings.contains("Invalid PATH_FACTS value for `PATH_FACTS_LIMIT=invalid`"),
            "{}",
            warnings
        );
        assert!(warnings.contains(EnvKey::Limit.hint()), "{}", warnings);
    }

    #[test]
    fn negative_integer() {
        let mut io = Vec::<u8>::new();
        let config = EnvConfig::from([(os("PATH_FACTS_LIMIT"), os("-1"))], &mut io);
        assert_eq!(config.dir_limit, EnvConfig::default().dir_limit);

        let warnings = String::from_utf8_lossy(&io).into_owned();
        assert!(
            warnings.contains("Invalid PATH_FACTS value for `PATH_FACTS_LIMIT=-1`"),
            "{}",
            warnings
        );
        assert!(warnings.contains(EnvKey::Limit.hint()), "{}", warnings);
    }

    #[test]
    fn valid_values() {
        for number in [0, 1, 42] {
            let mut io = Vec::<u8>::new();
            let config =
                EnvConfig::from([(os("PATH_FACTS_LIMIT"), os(&number.to_string()))], &mut io);
            assert_eq!(config.dir_limit, number);

            let warnings = String::from_utf8_lossy(&io).into_owned();
            assert!(warnings.is_empty(), "{}", warnings);
        }
    }

    #[test]
    fn usize_max() {
        for max in ["max", "MAX"] {
            let mut io = Vec::<u8>::new();
            let config = EnvConfig::from([(os("PATH_FACTS_LIMIT"), os(max))], &mut io);
            assert_eq!(config.dir_limit, usize::MAX);

            let warnings = String::from_utf8_lossy(&io).into_owned();
            assert!(
                warnings.is_empty(),
                "Warnings: {}. From value: {}",
                warnings,
                max
            );
        }
    }
}
