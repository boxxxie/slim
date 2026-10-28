use std::fmt::{Display, Formatter};

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug,
            Default,
            Clone,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub CompactString);

        impl $name {
            pub fn new(value: impl Into<CompactString>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0.as_str())
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(CompactString::from(value))
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(CompactString::from(value))
            }
        }

        impl From<CompactString> for $name {
            fn from(value: CompactString) -> Self {
                Self(value)
            }
        }
    };
}

define_id!(ProjectId);
define_id!(IssueId);
define_id!(EventId);
define_id!(ActivityId);

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProjectScope {
    #[default]
    All,
    Project(CompactString),
}

impl ProjectScope {
    pub fn label(&self) -> &str {
        match self {
            ProjectScope::All => "All",
            ProjectScope::Project(slug) => slug.as_str(),
        }
    }
}
