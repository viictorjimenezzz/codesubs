mod account;
mod gateway;
mod login;
mod quota;
mod reset;

pub use account::*;
pub use gateway::*;
pub use login::*;
pub use quota::*;
pub use reset::*;

pub(crate) mod optional_datetime {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<DateTime<Utc>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true))
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<DateTime<Utc>>, D::Error> {
        Option::deserialize(deserializer)
    }
}
