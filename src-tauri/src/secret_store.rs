use keyring::Entry;

const SERVICE: &str = "dev.notice.desktop";

pub fn set_secret(key: &str, value: &str) -> anyhow::Result<()> {
    let entry = Entry::new(SERVICE, key)?;
    entry.set_password(value)?;
    Ok(())
}

pub fn get_secret(key: &str) -> anyhow::Result<Option<String>> {
    let entry = Entry::new(SERVICE, key)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
