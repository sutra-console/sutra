//! At-rest encryption for workspace secrets ("Security" subsystem).
//!
//! A workspace's secret files (`keys.json`, `networks.json` + `.bak`, `macros.json`)
//! are bundled into one **age** file `.sutra/secrets.age`, encrypted to a set of
//! **public recipients** (always including this app's key). Two layers, kept apart:
//!
//! - **Vault** = the secrets, encrypted to X25519 *public* keys only. Because writes
//!   only need public keys (read from `security.json`), re-encrypting on every save
//!   needs no retained secret — just the session's decrypted member map.
//! - **App identity** = an X25519 keypair generated in-app and stored machine-local in
//!   the app config dir (NEVER in the workspace, so the `.sutra/` folder holds zero key
//!   material). It is what *unlocks* the vault. Stored cleartext (`identity.txt`, silent
//!   auto-unlock) or — when a password is set — scrypt-encrypted (`identity.age`, prompts).
//!
//! The password therefore protects the unlock credential, never the vault directly: age
//! forbids mixing a passphrase recipient with public-key recipients, and this keeps the
//! vault re-encryptable on write without ever holding the passphrase.
//!
//! Sharing (Phase B) = add a collaborator's public key to the vault recipients.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use age::secrecy::{ExposeSecret, SecretString};
use tauri::{AppHandle, Manager};

/// The workspace files we encrypt. Order is stable for deterministic packing.
const SECRET_FILES: &[&str] = &["keys.json", "networks.json", "networks.json.bak", "macros.json"];

const VAULT_FILE: &str = "secrets.age";
const SECURITY_FILE: &str = "security.json";

fn vault_path(dot: &Path) -> PathBuf {
    dot.join(VAULT_FILE)
}
fn security_path(dot: &Path) -> PathBuf {
    dot.join(SECURITY_FILE)
}

// ---- session state (Tauri-managed) -----------------------------------------

/// The decrypted secrets held while a workspace is unlocked. (The unlocking identity
/// isn't retained — password changes / re-keys recover it from machine-local storage.)
struct Unlocked {
    members: BTreeMap<String, Vec<u8>>, // filename → cleartext bytes
}

/// Session vault state. `None` = locked (or no vault / plaintext mode).
#[derive(Default)]
pub struct Vault {
    inner: Mutex<Option<Unlocked>>,
}

// ---- public on-disk config (security.json — non-secret, commit-safe) -------

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct RecipientCfg {
    kind: String, // "app" | "ssh" | "age"  (only "app" used in Phase A)
    pubkey: String,
    #[serde(default)]
    label: String,
}
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct SecurityCfg {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    recipients: Vec<RecipientCfg>,
    #[serde(default)]
    git_track_vault: bool,
    #[serde(default)]
    git_track_captures: bool,
    #[serde(default)]
    git_hooks: bool, // Phase B
}

fn load_cfg(dot: &Path) -> SecurityCfg {
    std::fs::read_to_string(security_path(dot))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
fn save_cfg(dot: &Path, cfg: &SecurityCfg) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(security_path(dot), json).map_err(|e| e.to_string())
}

/// Status surfaced to the UI's Security panel.
#[derive(serde::Serialize, Default)]
pub struct SecurityStatus {
    pub has_workspace: bool,
    pub enabled: bool,       // encryption configured for this workspace
    pub vault_present: bool, // secrets.age exists on disk
    pub unlocked: bool,      // session holds the decrypted members
    pub has_password: bool,  // the app identity is password-protected (identity.age, no cleartext)
    pub app_key_pub: String, // "age1…" public key (empty if no app key yet)
    pub git_track_vault: bool,
    pub git_track_captures: bool,
}

// ---- crypto primitives (standalone + unit-testable) ------------------------

fn pack(members: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    serde_json::to_vec(members).expect("member map serializes")
}
fn unpack(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("corrupt vault payload: {e}"))
}

/// Encrypt `plaintext` to a set of X25519 public recipients (age, binary output).
fn encrypt_to(recipients: &[age::x25519::Recipient], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    if recipients.is_empty() {
        return Err("no recipients to encrypt to".into());
    }
    let recs: Vec<&dyn age::Recipient> = recipients.iter().map(|r| r as &dyn age::Recipient).collect();
    let enc = age::Encryptor::with_recipients(recs.into_iter()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut w = enc.wrap_output(&mut out).map_err(|e| e.to_string())?;
    w.write_all(plaintext).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Decrypt an age blob with a single X25519 identity.
fn decrypt_with(id: &age::x25519::Identity, blob: &[u8]) -> Result<Vec<u8>, String> {
    age::decrypt(id, blob).map_err(|e| e.to_string())
}

fn id_to_string(id: &age::x25519::Identity) -> String {
    id.to_string().expose_secret().to_string()
}
fn id_pub(id: &age::x25519::Identity) -> String {
    id.to_public().to_string()
}
fn parse_identity(s: &str) -> Result<age::x25519::Identity, String> {
    s.trim().parse().map_err(|e: &str| e.to_string())
}
fn parse_recipient(s: &str) -> Result<age::x25519::Recipient, String> {
    s.trim().parse().map_err(|e: &str| e.to_string())
}

/// scrypt-encrypt the app identity under a passphrase (the password-protected store).
fn wrap_identity(passphrase: &str, id: &age::x25519::Identity) -> Result<Vec<u8>, String> {
    let enc = age::Encryptor::with_user_passphrase(SecretString::from(passphrase.to_owned()));
    let mut out = Vec::new();
    let mut w = enc.wrap_output(&mut out).map_err(|e| e.to_string())?;
    w.write_all(id_to_string(id).as_bytes()).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    Ok(out)
}
/// Decrypt a scrypt-wrapped identity blob with a passphrase.
fn unwrap_identity(passphrase: &str, blob: &[u8]) -> Result<age::x25519::Identity, String> {
    let dec = age::Decryptor::new_buffered(blob).map_err(|_| "wrong password".to_string())?;
    let scrypt_id = age::scrypt::Identity::new(SecretString::from(passphrase.to_owned()));
    let mut reader = dec
        .decrypt(std::iter::once(&scrypt_id as &dyn age::Identity))
        .map_err(|_| "wrong password".to_string())?;
    let mut s = String::new();
    reader.read_to_string(&mut s).map_err(|e| e.to_string())?;
    parse_identity(&s)
}

// ---- app identity (machine-local, in the app config dir) -------------------

fn id_cleartext_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("identity.txt"))
}
fn id_scrypt_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("identity.age"))
}

/// Whether the app identity is password-protected (scrypt store present, no cleartext).
fn has_password(app: &AppHandle) -> bool {
    id_scrypt_path(app).is_some_and(|p| p.exists())
        && !id_cleartext_path(app).is_some_and(|p| p.exists())
}

/// Load the app identity from cleartext storage (None if absent or password-protected).
fn load_cleartext_identity(app: &AppHandle) -> Option<age::x25519::Identity> {
    let p = id_cleartext_path(app)?;
    let s = std::fs::read_to_string(p).ok()?;
    parse_identity(&s).ok()
}

/// The app identity, generating + persisting (cleartext) one on first use.
fn ensure_app_identity(app: &AppHandle) -> Result<age::x25519::Identity, String> {
    if let Some(id) = load_cleartext_identity(app) {
        return Ok(id);
    }
    if has_password(app) {
        return Err("app key is password-protected — unlock first".into());
    }
    let id = age::x25519::Identity::generate();
    let p = id_cleartext_path(app).ok_or("no app config dir")?;
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&p, id_to_string(&id)).map_err(|e| e.to_string())?;
    Ok(id)
}

/// The app's public key ("age1…"), if an identity exists (cleartext or scrypt).
pub fn app_key_pub(app: &AppHandle) -> String {
    if let Some(id) = load_cleartext_identity(app) {
        return id_pub(&id);
    }
    // Password-protected: recover the pubkey from the workspace recipient list.
    if let Some(dot) = crate::workspace::dot_sutra_existing(app) {
        if let Some(r) = load_cfg(&dot).recipients.iter().find(|r| r.kind == "app") {
            return r.pubkey.clone();
        }
    }
    String::new()
}

// ---- recipients ------------------------------------------------------------

/// The X25519 recipients the vault is (re-)encrypted to, from `security.json`.
fn load_recipients(dot: &Path) -> Result<Vec<age::x25519::Recipient>, String> {
    let cfg = load_cfg(dot);
    let mut out = Vec::new();
    for r in &cfg.recipients {
        // Phase A only handles age (X25519) recipients; ssh recipients arrive in Phase B.
        if let Ok(rec) = parse_recipient(&r.pubkey) {
            out.push(rec);
        }
    }
    if out.is_empty() {
        return Err("no recipients configured".into());
    }
    Ok(out)
}

// ---- session helpers -------------------------------------------------------

fn set_session(app: &AppHandle, members: BTreeMap<String, Vec<u8>>) {
    *app.state::<Vault>().inner.lock().unwrap() = Some(Unlocked { members });
}
fn clear_session(app: &AppHandle) {
    *app.state::<Vault>().inner.lock().unwrap() = None;
}

// ---- secret IO (the boundary workspace.rs + serial.rs go through) ----------

/// Read a secret file's bytes. Encrypted workspace ⇒ from the unlocked session (None
/// if locked); otherwise the plaintext file on disk.
pub fn read_secret(app: &AppHandle, dot: &Path, name: &str) -> Option<Vec<u8>> {
    if vault_path(dot).exists() {
        let v = app.state::<Vault>();
        let g = v.inner.lock().unwrap();
        g.as_ref().and_then(|u| u.members.get(name).cloned())
    } else {
        std::fs::read(dot.join(name)).ok()
    }
}

/// Write a secret file. Encrypted workspace ⇒ update the session + re-encrypt the vault
/// (errors if locked); otherwise a plaintext write.
pub fn write_secret(app: &AppHandle, dot: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    if vault_path(dot).exists() {
        let recipients = load_recipients(dot)?;
        let v = app.state::<Vault>();
        let mut g = v.inner.lock().unwrap();
        let u = g.as_mut().ok_or("workspace is locked")?;
        u.members.insert(name.to_string(), data.to_vec());
        let blob = encrypt_to(&recipients, &pack(&u.members))?;
        std::fs::write(vault_path(dot), blob).map_err(|e| e.to_string())
    } else {
        let _ = std::fs::create_dir_all(dot);
        std::fs::write(dot.join(name), data).map_err(|e| e.to_string())
    }
}

// ---- lifecycle (called by Tauri commands) ---------------------------------

/// Encrypt the workspace's plaintext secrets into the vault. Generates the app key if
/// needed, optionally sets a password, removes the plaintext files, unlocks the session.
pub fn enable(app: &AppHandle, dot: &Path, password: Option<String>) -> Result<(), String> {
    if vault_path(dot).exists() {
        return Err("already encrypted".into());
    }
    let _ = std::fs::create_dir_all(dot);
    let identity = ensure_app_identity(app)?;

    // Absorb existing plaintext secret files into the member map.
    let mut members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for &name in SECRET_FILES {
        if let Ok(bytes) = std::fs::read(dot.join(name)) {
            members.insert(name.to_string(), bytes);
        }
    }

    // Config: the app key is the sole recipient (collaborators added in Phase B).
    let mut cfg = load_cfg(dot);
    cfg.enabled = true;
    if !cfg.recipients.iter().any(|r| r.kind == "app") {
        cfg.recipients.push(RecipientCfg {
            kind: "app".into(),
            pubkey: id_pub(&identity),
            label: "this app".into(),
        });
    }
    save_cfg(dot, &cfg)?;

    let recipients = load_recipients(dot)?;
    let blob = encrypt_to(&recipients, &pack(&members))?;
    std::fs::write(vault_path(dot), blob).map_err(|e| e.to_string())?;

    // Plaintext is now redundant — remove it.
    for &name in SECRET_FILES {
        let _ = std::fs::remove_file(dot.join(name));
    }
    if let Some(pw) = password.filter(|p| !p.is_empty()) {
        set_app_password_inner(app, &identity, &pw)?;
    }
    set_session(app, members);
    Ok(())
}

/// Decrypt the vault back to plaintext files and remove the vault. Requires unlock.
pub fn disable(app: &AppHandle, dot: &Path) -> Result<(), String> {
    if !vault_path(dot).exists() {
        return Ok(()); // already plaintext
    }
    let v = app.state::<Vault>();
    let members = {
        let g = v.inner.lock().unwrap();
        g.as_ref().ok_or("unlock before disabling encryption")?.members.clone()
    };
    for (name, bytes) in &members {
        std::fs::write(dot.join(name), bytes).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(vault_path(dot));
    let mut cfg = load_cfg(dot);
    cfg.enabled = false;
    save_cfg(dot, &cfg)?;
    clear_session(app);
    Ok(())
}

/// Unlock the vault: cleartext app key (no password) or password-derived app key.
pub fn unlock(app: &AppHandle, dot: &Path, password: Option<String>) -> Result<(), String> {
    if !vault_path(dot).exists() {
        return Ok(()); // nothing to unlock
    }
    let identity = match load_cleartext_identity(app) {
        Some(id) => id,
        None => {
            let pw = password.filter(|p| !p.is_empty()).ok_or("password required")?;
            let blob = id_scrypt_path(app)
                .filter(|p| p.exists())
                .and_then(|p| std::fs::read(p).ok())
                .ok_or("no app key on this machine")?;
            unwrap_identity(&pw, &blob)?
        }
    };
    let blob = std::fs::read(vault_path(dot)).map_err(|e| e.to_string())?;
    let members = unpack(&decrypt_with(&identity, &blob)?)?;
    set_session(app, members);
    Ok(())
}

/// Forget the decrypted session (the workspace becomes locked).
pub fn lock(app: &AppHandle) {
    clear_session(app);
}

/// Try a silent unlock with the cleartext app key (used on workspace open). Returns
/// false (without erroring) when a password is required or there's no vault/key.
pub fn auto_unlock(app: &AppHandle, dot: &Path) -> bool {
    if !vault_path(dot).exists() {
        return false;
    }
    unlock(app, dot, None).is_ok()
}

fn set_app_password_inner(app: &AppHandle, id: &age::x25519::Identity, new_pw: &str) -> Result<(), String> {
    let blob = wrap_identity(new_pw, id)?;
    let scrypt = id_scrypt_path(app).ok_or("no app config dir")?;
    if let Some(parent) = scrypt.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&scrypt, blob).map_err(|e| e.to_string())?;
    if let Some(clear) = id_cleartext_path(app) {
        let _ = std::fs::remove_file(clear); // no cleartext copy once password-protected
    }
    Ok(())
}

/// Set, change, or clear the app password. `new` empty/None clears it (back to cleartext).
pub fn set_password(app: &AppHandle, old: Option<String>, new: Option<String>) -> Result<(), String> {
    // Recover the identity using whatever store currently exists.
    let identity = match load_cleartext_identity(app) {
        Some(id) => id,
        None => {
            let pw = old.filter(|p| !p.is_empty()).ok_or("current password required")?;
            let blob = id_scrypt_path(app)
                .filter(|p| p.exists())
                .and_then(|p| std::fs::read(p).ok())
                .ok_or("no app key on this machine")?;
            unwrap_identity(&pw, &blob)?
        }
    };
    match new.filter(|p| !p.is_empty()) {
        Some(pw) => set_app_password_inner(app, &identity, &pw),
        None => {
            // Clear the password: write cleartext, drop the scrypt store.
            let clear = id_cleartext_path(app).ok_or("no app config dir")?;
            if let Some(parent) = clear.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&clear, id_to_string(&identity)).map_err(|e| e.to_string())?;
            if let Some(scrypt) = id_scrypt_path(app) {
                let _ = std::fs::remove_file(scrypt);
            }
            Ok(())
        }
    }
}

/// Generate a fresh app key. If an encrypted workspace exists it is re-keyed to the
/// new key (requires unlock); refused while the app key is password-protected (clear
/// the password first, since re-wrapping needs the passphrase).
pub fn regenerate_app_key(app: &AppHandle, dot: Option<&Path>) -> Result<(), String> {
    if has_password(app) {
        return Err("clear the app password before regenerating the key".into());
    }
    let new_id = age::x25519::Identity::generate();
    let clear = id_cleartext_path(app).ok_or("no app config dir")?;
    if let Some(parent) = clear.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&clear, id_to_string(&new_id)).map_err(|e| e.to_string())?;

    if let Some(dot) = dot {
        if vault_path(dot).exists() {
            let v = app.state::<Vault>();
            let members = {
                let g = v.inner.lock().unwrap();
                g.as_ref().ok_or("unlock before regenerating the app key")?.members.clone()
            };
            let mut cfg = load_cfg(dot);
            cfg.recipients.retain(|r| r.kind != "app");
            cfg.recipients.push(RecipientCfg {
                kind: "app".into(),
                pubkey: id_pub(&new_id),
                label: "this app".into(),
            });
            save_cfg(dot, &cfg)?;
            let recipients = load_recipients(dot)?;
            let blob = encrypt_to(&recipients, &pack(&members))?;
            std::fs::write(vault_path(dot), blob).map_err(|e| e.to_string())?;
            set_session(app, members);
        }
    }
    Ok(())
}

/// Set the git-tracking toggles in `security.json` (the caller re-runs the .gitignore).
pub fn set_git_track(dot: &Path, vault: Option<bool>, captures: Option<bool>) -> Result<(), String> {
    let mut cfg = load_cfg(dot);
    if let Some(v) = vault {
        cfg.git_track_vault = v;
    }
    if let Some(c) = captures {
        cfg.git_track_captures = c;
    }
    save_cfg(dot, &cfg)
}

/// What the managed `.gitignore` block should ignore, given the workspace's config.
/// `(ignore_secrets_plaintext, ignore_vault, ignore_captures)`.
pub fn gitignore_flags(dot: &Path) -> (bool, bool, bool) {
    let cfg = load_cfg(dot);
    let encrypted = vault_path(dot).exists();
    (
        !encrypted,           // plaintext secret files only exist when not encrypted
        !cfg.git_track_vault, // ignore the vault unless the user opted it into git
        !cfg.git_track_captures,
    )
}

/// Status for the Security panel.
pub fn status(app: &AppHandle, dot: Option<&Path>) -> SecurityStatus {
    let Some(dot) = dot else {
        return SecurityStatus { has_workspace: false, ..Default::default() };
    };
    let cfg = load_cfg(dot);
    let vault_present = vault_path(dot).exists();
    let unlocked = app.state::<Vault>().inner.lock().unwrap().is_some();
    SecurityStatus {
        has_workspace: true,
        enabled: cfg.enabled || vault_present,
        vault_present,
        unlocked,
        has_password: has_password(app),
        app_key_pub: app_key_pub(app),
        git_track_vault: cfg.git_track_vault,
        git_track_captures: cfg.git_track_captures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x25519_member_roundtrip() {
        let id = age::x25519::Identity::generate();
        let mut members = BTreeMap::new();
        members.insert("networks.json".to_string(), b"{\"key\":\"deadbeef\"}".to_vec());
        let blob = encrypt_to(&[id.to_public()], &pack(&members)).unwrap();
        let back = unpack(&decrypt_with(&id, &blob).unwrap()).unwrap();
        assert_eq!(back.get("networks.json").unwrap(), members.get("networks.json").unwrap());
    }

    #[test]
    fn wrong_identity_fails() {
        let id = age::x25519::Identity::generate();
        let other = age::x25519::Identity::generate();
        let blob = encrypt_to(&[id.to_public()], b"secret").unwrap();
        assert!(decrypt_with(&other, &blob).is_err());
    }

    #[test]
    fn passphrase_identity_roundtrip() {
        let id = age::x25519::Identity::generate();
        let pub_before = id_pub(&id);
        let blob = wrap_identity("hunter2", &id).unwrap();
        let back = unwrap_identity("hunter2", &blob).unwrap();
        assert_eq!(id_pub(&back), pub_before);
        assert!(unwrap_identity("wrong", &blob).is_err());
    }

    #[test]
    fn identity_string_roundtrip() {
        let id = age::x25519::Identity::generate();
        let s = id_to_string(&id);
        let back = parse_identity(&s).unwrap();
        assert_eq!(id_pub(&back), id_pub(&id));
        // the public key parses as a recipient
        assert!(parse_recipient(&id_pub(&id)).is_ok());
    }
}
