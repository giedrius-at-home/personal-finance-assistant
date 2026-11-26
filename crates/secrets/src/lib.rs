//! The app's secrets (tokens, keys, bank consents), each in its own file in
//! `~/Library/Application Support/Finance/secrets/`: the folder is readable
//! by the owner only (0700) and every file too (0600), as the old bank
//! importer kept the same secrets. The Keychain was dropped because every new
//! (ad-hoc signed) build of the app had to be approved again with the
//! keychain password. In memory secrets are [`Secret`]s: wiped on drop and
//! never printed by `Debug`.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use zeroize::Zeroizing;

/// Secrets the app keeps.
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
    /// The SMTP (app) password the daily spending email signs in with.
    SmtpPassword,
    /// The TypeSafe API key for Jev category suggestions.
    TypeSafeApiKey,
    /// The Mistral AI key: category suggestions, Gisel's words and voice.
    MistralApiKey,
    /// The token the Finance server asks of the app and the phone.
    ServerToken,
    /// The server's Web Push (VAPID) signing key, PKCS#8 in base64url; made
    /// on first use.
    VapidKey,
    /// Gisel's Telegram bot token (from @BotFather).
    TelegramGisel,
    /// Rafael's Telegram bot token.
    TelegramRafael,
    /// The GIPHY API key for the reaction GIF after the Telegram brief.
    GiphyApiKey,
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
    fn file(self) -> &'static str {
        match self {
            Self::ReleaseToken => "github-release-token",
            Self::EnableBankingAppId => "enable-banking-app-id",
            Self::EnableBankingKey => "enable-banking-private-key",
            Self::BankConsent(Bank::AbnAmro) => "bank-consent-abnamro",
            Self::BankConsent(Bank::Revolut) => "bank-consent-revolut",
            Self::BankConsent(Bank::Swedbank) => "bank-consent-swedbank",
            Self::SmtpPassword => "smtp-password",
            Self::TypeSafeApiKey => "typesafe-api-key",
            Self::MistralApiKey => "mistral-api-key",
            Self::ServerToken => "server-token",
            Self::VapidKey => "web-push-vapid-key",
            Self::TelegramGisel => "telegram-bot-gisel",
            Self::TelegramRafael => "telegram-bot-rafael",
            Self::GiphyApiKey => "giphy-api-key",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("secret store: {0}")]
    Io(#[from] std::io::Error),
    #[error("stored secret is not text")]
    NotText,
    #[error("no home folder")]
    NoHome,
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

/// The secrets folder: `FINANCE_SECRETS_DIR` when set (tests), else
/// `~/Library/Application Support/Finance/secrets`.
fn dir() -> Result<PathBuf, Error> {
    if let Some(dir) = std::env::var_os("FINANCE_SECRETS_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").ok_or(Error::NoHome)?;
    Ok(PathBuf::from(home).join("Library/Application Support/Finance/secrets"))
}

#[cfg(unix)]
fn private_dir(path: &std::path::Path) -> Result<(), Error> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn private_dir(path: &std::path::Path) -> Result<(), Error> {
    fs::create_dir_all(path)?;
    Ok(())
}

pub fn get(item: Item) -> Result<Option<Secret>, Error> {
    get_in(&dir()?, item)
}

pub fn set(item: Item, value: &Secret) -> Result<(), Error> {
    set_in(&dir()?, item, value)
}

pub fn delete(item: Item) -> Result<(), Error> {
    delete_in(&dir()?, item)
}

fn get_in(dir: &std::path::Path, item: Item) -> Result<Option<Secret>, Error> {
    match fs::read(dir.join(item.file())) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            let text = std::str::from_utf8(&bytes).map_err(|_| Error::NotText)?;
            Ok(Some(Secret::new(text.to_owned())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Write `value` to a fresh owner-only file, then move it into place, so a
/// reader never sees half a secret.
fn set_in(dir: &std::path::Path, item: Item, value: &Secret) -> Result<(), Error> {
    private_dir(dir)?;
    let path = dir.join(item.file());
    let temporary = dir.join(format!(".{}.tmp", item.file()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(value.expose().as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, &path)?;
    Ok(())
}

fn delete_in(dir: &std::path::Path, item: Item) -> Result<(), Error> {
    match fs::remove_file(dir.join(item.file())) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_owner_only_files() {
        let dir = std::env::temp_dir()
            .join(format!("finance-secrets-{}", std::process::id()))
            .join("secrets");
        let _ = fs::remove_dir_all(&dir);
        let item = Item::BankConsent(Bank::Revolut);
        assert!(get_in(&dir, item).unwrap().is_none());
        set_in(&dir, item, &Secret::new("first".into())).unwrap();
        set_in(&dir, item, &Secret::new("second".into())).unwrap();
        assert_eq!(get_in(&dir, item).unwrap().unwrap().expose(), "second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join("bank-consent-revolut")), 0o600);
        }
        delete_in(&dir, item).unwrap();
        delete_in(&dir, item).unwrap();
        assert!(get_in(&dir, item).unwrap().is_none());
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }
}
