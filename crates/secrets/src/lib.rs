//! Secrets live in the macOS Keychain under the app's service name. In memory
//! they are [`Secret`]s: wiped on drop and never printed by `Debug`.

use std::fmt;

use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
const SERVICE: &str = "local.giedrius.finance";

/// Keychain items the app uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// Fine-grained, read-only GitHub token for downloading releases.
    ReleaseToken,
    /// The Enable Banking application id.
    EnableBankingAppId,
    /// The Enable Banking application's PEM private key.
    EnableBankingKey,
    /// A bank's consent (sessions and account mapping), by bank id.
    BankConsent(Bank),
    /// The TypeSafe API key for Jev category suggestions.
    TypeSafeApiKey,
}

/// The banks with a stored consent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bank {
    AbnAmro,
    Revolut,
    Swedbank,
}

impl Bank {
    /// The bank with id `id` (`abnamro`, `revolut`, `swedbank`).
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "abnamro" => Some(Self::AbnAmro),
            "revolut" => Some(Self::Revolut),
            "swedbank" => Some(Self::Swedbank),
            _ => None,
        }
    }
}

impl Item {
    #[cfg(target_os = "macos")]
    fn account(self) -> &'static str {
        match self {
            Self::ReleaseToken => "github-release-token",
            Self::EnableBankingAppId => "enable-banking-app-id",
            Self::EnableBankingKey => "enable-banking-private-key",
            Self::BankConsent(Bank::AbnAmro) => "bank-consent-abnamro",
            Self::BankConsent(Bank::Revolut) => "bank-consent-revolut",
            Self::BankConsent(Bank::Swedbank) => "bank-consent-swedbank",
            Self::TypeSafeApiKey => "typesafe-api-key",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("keychain error {0}")]
    Keychain(i32),
    #[error("stored secret is not text")]
    NotText,
    #[error("the keychain is only available on macOS")]
    Unsupported,
}

/// A secret value that is zeroed when dropped.
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use security_framework::passwords;
    use zeroize::Zeroizing;

    use super::{Error, Item, SERVICE, Secret};

    /// errSecItemNotFound
    const NOT_FOUND: i32 = -25300;

    pub fn get(item: Item) -> Result<Option<Secret>, Error> {
        match passwords::get_generic_password(SERVICE, item.account()) {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                let text = std::str::from_utf8(&bytes).map_err(|_| Error::NotText)?;
                Ok(Some(Secret::new(text.to_owned())))
            }
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(Error::Keychain(e.code())),
        }
    }

    pub fn set(item: Item, value: &Secret) -> Result<(), Error> {
        passwords::set_generic_password(SERVICE, item.account(), value.expose().as_bytes())
            .map_err(|e| Error::Keychain(e.code()))
    }

    pub fn delete(item: Item) -> Result<(), Error> {
        match passwords::delete_generic_password(SERVICE, item.account()) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Err(e) => Err(Error::Keychain(e.code())),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    use super::{Error, Item, Secret};

    pub fn get(_: Item) -> Result<Option<Secret>, Error> {
        Err(Error::Unsupported)
    }
    pub fn set(_: Item, _: &Secret) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    pub fn delete(_: Item) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
}

pub use keychain::{delete, get, set};
