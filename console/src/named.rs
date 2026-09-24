//! Closed sets that something outside names in words — the CLI, the task service —
//! and that may one day name something new. Each is an enum with an `Unknown`
//! variant that keeps the word, so a new value is visible rather than dropped, and
//! is logged where it arrived.

use serde::Deserialize;
use serde::de::value::{Error, StrDeserializer};
use serde::de::{DeserializeOwned, Deserializer};

/// An enum whose values the outside world names in words.
pub trait Named: DeserializeOwned {
    /// The value for a word none of the others is named by.
    fn unknown(name: String) -> Self;
}

/// The value `name` stands for, or the unknown one carrying it.
pub fn named<T: Named>(name: &str) -> T {
    T::deserialize(StrDeserializer::<Error>::new(name)).unwrap_or_else(|_| {
        tracing::warn!("{} has no value named {name:?}", std::any::type_name::<T>());
        T::unknown(name.to_owned())
    })
}

/// [`named`], for a field read with `#[serde(deserialize_with = "...")]`.
pub fn by_name<'de, D: Deserializer<'de>, T: Named>(from: D) -> Result<T, D::Error> {
    Ok(named(&String::deserialize(from)?))
}
