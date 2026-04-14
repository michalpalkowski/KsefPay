use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// `KSeF` environment selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KSeFEnvironment {
    Test,
    Demo,
    Production,
}

impl KSeFEnvironment {
    #[must_use]
    pub fn api_base_url(self) -> &'static str {
        match self {
            Self::Test => "https://api-test.ksef.mf.gov.pl/api/v2",
            Self::Demo => "https://api-demo.ksef.mf.gov.pl/api/v2",
            Self::Production => "https://api.ksef.mf.gov.pl/api/v2",
        }
    }
}

impl fmt::Display for KSeFEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Test => write!(f, "test"),
            Self::Demo => write!(f, "demo"),
            Self::Production => write!(f, "production"),
        }
    }
}

impl FromStr for KSeFEnvironment {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "test" => Ok(Self::Test),
            "demo" => Ok(Self::Demo),
            "production" | "prod" => Ok(Self::Production),
            other => Err(DomainError::InvalidParse {
                type_name: "KSeFEnvironment",
                value: other.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_urls_are_correct() {
        assert_eq!(
            KSeFEnvironment::Test.api_base_url(),
            "https://api-test.ksef.mf.gov.pl/api/v2"
        );
        assert_eq!(
            KSeFEnvironment::Demo.api_base_url(),
            "https://api-demo.ksef.mf.gov.pl/api/v2"
        );
        assert_eq!(
            KSeFEnvironment::Production.api_base_url(),
            "https://api.ksef.mf.gov.pl/api/v2"
        );
    }

    #[test]
    fn display_and_from_str_round_trip() {
        for env in [
            KSeFEnvironment::Test,
            KSeFEnvironment::Demo,
            KSeFEnvironment::Production,
        ] {
            let s = env.to_string();
            let parsed: KSeFEnvironment = s.parse().unwrap();
            assert_eq!(env, parsed);
        }
    }

    #[test]
    fn from_str_accepts_prod_alias() {
        let env: KSeFEnvironment = "prod".parse().unwrap();
        assert_eq!(env, KSeFEnvironment::Production);
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert!("staging".parse::<KSeFEnvironment>().is_err());
    }
}
