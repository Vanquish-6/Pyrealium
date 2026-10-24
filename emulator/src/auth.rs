//! Password accounts and short-lived native GLS tickets. No password enters UDP.
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const TICKET_SECONDS: u64 = 60;
const MAX_ACCOUNTS: usize = 10_000;
const MAX_JOURNAL: u64 = 16 * 1024 * 1024;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| invalid("system clock precedes Unix epoch"))
}
pub fn valid_account(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
pub fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut value = [0; N];
    getrandom::getrandom(&mut value)
        .map_err(|_| invalid("operating-system randomness unavailable"))?;
    Ok(value)
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
pub fn login_proof(login: &crate::wire::Login) -> String {
    let mut digest = Sha256::new();
    for bytes in [
        login.version.as_bytes(),
        login.account.as_bytes(),
        &login.crypto_data,
        &login.extra_data,
    ] {
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    digest.update(login.auth_type.to_le_bytes());
    digest.update(login.auth_flags.to_le_bytes());
    hex(&digest.finalize())
}
pub fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
fn private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub struct Lock(PathBuf);
impl Lock {
    fn acquire(path: PathBuf) -> io::Result<Self> {
        let mut file = private_file(&path)?;
        let lock = Self(path);
        writeln!(file, "{}", std::process::id())?;
        file.sync_all()?;
        Ok(lock)
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    name: String,
    password_hash: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ticket {
    account: String,
    ip: IpAddr,
    expires: u64,
}

pub struct Accounts {
    _lock: Lock,
    file: File,
    records: BTreeMap<String, String>,
    tickets: PathBuf,
    dummy_hash: String,
    poisoned: bool,
}
fn password_hash(password: &str) -> io::Result<String> {
    let salt =
        SaltString::encode_b64(&random::<16>()?).map_err(|_| invalid("password salt encoding"))?;
    let params =
        argon2::Params::new(19456, 2, 1, Some(32)).map_err(|_| invalid("password parameters"))?;
    Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| invalid("password hashing failed"))
}
impl Accounts {
    pub fn open(directory: &Path) -> io::Result<Self> {
        private_directory(directory)?;
        let guard = Lock::acquire(directory.join("accounts.lock"))?;
        let tickets = directory.join("tickets");
        private_directory(&tickets)?;
        let path = directory.join("accounts.journal");
        if !path.exists() {
            private_file(&path)?.sync_all()?;
        }
        let mut file = OpenOptions::new().read(true).append(true).open(&path)?;
        let mut bytes = Vec::new();
        (&mut file).take(MAX_JOURNAL + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_JOURNAL || (!bytes.is_empty() && bytes.last() != Some(&b'\n')) {
            return Err(invalid("invalid account journal size or torn tail"));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| invalid("account journal encoding"))?;
        let mut records = BTreeMap::new();
        for line in text.lines() {
            if line.len() > 1024 || records.len() >= MAX_ACCOUNTS {
                return Err(invalid("account journal capacity"));
            }
            let (body, checksum) = line
                .rsplit_once('\t')
                .ok_or_else(|| invalid("account journal framing"))?;
            if digest(body.as_bytes()) != checksum {
                return Err(invalid("account journal checksum"));
            }
            let record: Account =
                serde_json::from_str(body).map_err(|_| invalid("account journal record"))?;
            if !valid_account(&record.name)
                || !record
                    .password_hash
                    .starts_with("$argon2id$v=19$m=19456,t=2,p=1$")
                || record.password_hash.len() > 256
                || PasswordHash::new(&record.password_hash).is_err()
                || records.insert(record.name, record.password_hash).is_some()
            {
                return Err(invalid("invalid or duplicate account"));
            }
        }
        Ok(Self {
            _lock: guard,
            file,
            records,
            tickets,
            dummy_hash: password_hash(&hex(&random::<32>()?))?,
            poisoned: false,
        })
    }
    pub fn create(&mut self, name: &str, password: &str) -> io::Result<()> {
        if !valid_account(name)
            || !(12..=128).contains(&password.chars().count())
            || password.len() > 512
        {
            return Err(invalid(
                "Use a lowercase account name of 1-32 letters, digits or underscores and a password of 12-128 characters.",
            ));
        }
        if self.poisoned || self.records.len() >= MAX_ACCOUNTS {
            return Err(invalid("Account registration is unavailable."));
        }
        if self.records.contains_key(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "That account name is unavailable.",
            ));
        }
        let record = Account {
            name: name.into(),
            password_hash: password_hash(password)?,
        };
        let body = serde_json::to_string(&record).map_err(|_| invalid("account encoding"))?;
        let line = format!("{body}\t{}\n", digest(body.as_bytes()));
        if self.file.metadata()?.len() + line.len() as u64 > MAX_JOURNAL {
            return Err(invalid("Account registration is unavailable."));
        }
        if self
            .file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_all())
            .is_err()
        {
            self.poisoned = true;
            return Err(invalid(
                "Account storage failed; restart and inspect the journal.",
            ));
        }
        self.records.insert(record.name, record.password_hash);
        Ok(())
    }
    pub fn verify(&self, name: &str, password: &str) -> bool {
        if self.poisoned || !valid_account(name) || password.len() > 512 {
            return false;
        }
        let stored = self.records.get(name);
        let parsed = PasswordHash::new(stored.unwrap_or(&self.dummy_hash));
        let matched = parsed.is_ok_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        });
        stored.is_some() && matched
    }
    pub fn issue(&self, account: &str, ip: IpAddr) -> io::Result<String> {
        if !self.records.contains_key(account) || !ip.is_ipv4() {
            return Err(invalid(
                "A registered account and IPv4 connection are required.",
            ));
        }
        let time = now()?;
        let mut count = 0;
        for entry in fs::read_dir(&self.tickets)?.take(2049) {
            let entry = entry?;
            count += 1;
            if count > 2048 {
                return Err(invalid("Login ticket capacity."));
            }
            let path = entry.path();
            if !matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("json" | "used")
            ) {
                continue;
            }
            let Ok(file) = File::open(&path) else {
                continue;
            };
            let mut bytes = Vec::new();
            file.take(2049).read_to_end(&mut bytes)?;
            if bytes.len() <= 2048
                && serde_json::from_slice::<Ticket>(&bytes)
                    .is_ok_and(|ticket| ticket.expires <= time)
            {
                let _ = fs::remove_file(path);
                count -= 1;
            }
        }
        if count >= 1024 {
            return Err(invalid("Login ticket capacity."));
        }
        let token = hex(&random::<32>()?);
        let path = self
            .tickets
            .join(format!("{}.json", digest(token.as_bytes())));
        let bytes = serde_json::to_vec(&Ticket {
            account: account.into(),
            ip,
            expires: time + TICKET_SECONDS,
        })
        .map_err(|_| invalid("ticket encoding"))?;
        let mut file = private_file(&path)?;
        file.write_all(&bytes).and_then(|()| file.sync_all())?;
        Ok(token)
    }
}

/// Atomically consume a SHA256-indexed, single-use ticket. Never log its value.
pub fn consume(directory: &Path, account: &str, ip: IpAddr, token: &str) -> io::Result<()> {
    if !valid_account(account)
        || token.len() != 64
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("Public login rejected."));
    }
    let stem = digest(token.as_bytes());
    let source = directory.join("tickets").join(format!("{stem}.json"));
    let claimed = source.with_extension("used");
    fs::rename(&source, &claimed).map_err(|_| invalid("Public login rejected."))?;
    let result = (|| {
        let mut bytes = Vec::new();
        File::open(&claimed)?.take(2049).read_to_end(&mut bytes)?;
        if bytes.len() > 2048 {
            return Err(invalid("Public login rejected."));
        }
        let ticket: Ticket =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Public login rejected."))?;
        let time = now()?;
        if ticket.account != account
            || ticket.ip != ip
            || ticket.expires <= time
            || ticket.expires > time.saturating_add(TICKET_SECONDS)
        {
            return Err(invalid("Public login rejected."));
        }
        Ok(())
    })();
    let _ = fs::remove_file(claimed);
    result.map_err(|_| invalid("Public login rejected."))
}

/// Bounded rolling windows for unauthenticated login work.
pub struct RateLimit {
    entries: BTreeMap<IpAddr, (u64, u32)>,
    limit: u32,
}
impl RateLimit {
    pub fn new(limit: u32) -> Self {
        Self {
            entries: BTreeMap::new(),
            limit,
        }
    }
    pub fn allow(&mut self, ip: IpAddr, seconds: u64) -> bool {
        self.entries
            .retain(|_, (start, _)| seconds.saturating_sub(*start) < 60);
        if !self.entries.contains_key(&ip) && self.entries.len() >= 2048 {
            return false;
        }
        let (_, count) = self.entries.entry(ip).or_insert((seconds, 0));
        if *count >= self.limit {
            return false;
        }
        *count += 1;
        true
    }
}
