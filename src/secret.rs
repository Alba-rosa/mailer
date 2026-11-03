//! Password storage in the operating system credential store
//! (macOS Keychain, Windows Credential Manager, Secret Service on Linux).

const SERVICE: &str = "net.albarosa.mailer";

fn entry(email: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, email).map_err(|e| e.to_string())
}

pub fn available() -> bool {
    keyring::Entry::store_status().is_ok()
}

pub fn load(email: &str) -> Option<String> {
    entry(email).ok()?.get_password().ok()
}

pub fn save(email: &str, password: &str) -> Result<(), String> {
    entry(email)?.set_password(password).map_err(|e| e.to_string())
}

pub fn delete(email: &str) {
    if let Ok(e) = entry(email) {
        let _ = e.delete_credential();
    }
}
