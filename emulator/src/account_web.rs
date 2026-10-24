//! Loopback account backend. A TLS reverse proxy owns public HTTPS.
use crate::auth::{self, Accounts, RateLimit};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{self, Read},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use zeroize::Zeroize;

pub struct Config {
    pub bind: SocketAddr,
    pub auth_data: PathBuf,
    pub game_data: PathBuf,
    pub origin: String,
    pub game_host: String,
    pub game_port: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    account: String,
    password: String,
}
impl Drop for Credentials {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}
fn bad(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text)
}
fn reserved(game_data: &Path, account: &str) -> io::Result<bool> {
    if matches!(account, "local" | "admin" | "administrator" | "system") {
        return Ok(true);
    }
    let path = game_data.join("characters.journal");
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    Ok(crate::store::decode_journal(&bytes)?
        .values()
        .any(|character| character.account.eq_ignore_ascii_case(account)))
}
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['-', '.'])
        && !host.ends_with(['-', '.'])
        && !host.contains("..")
        && host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
}
fn client_ip(request: &Request) -> io::Result<IpAddr> {
    let remote = request
        .remote_addr()
        .ok_or_else(|| bad("Missing source address."))?;
    if !remote.ip().is_loopback() {
        return Err(bad("Use the HTTPS account endpoint."));
    }
    let forwarded: Vec<_> = request
        .headers()
        .iter()
        .filter(|h| h.field.equiv("X-Forwarded-For"))
        .collect();
    let ip = match forwarded.as_slice() {
        [] => remote.ip(),
        [header] => header
            .value
            .as_str()
            .trim()
            .parse::<IpAddr>()
            .map_err(|_| bad("Invalid proxy source address."))?,
        _ => return Err(bad("Ambiguous proxy source address.")),
    };
    match ip {
        IpAddr::V4(_) => Ok(ip),
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .ok_or_else(|| bad("An IPv4 connection is required for this game server.")),
    }
}
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static valid HTTP header")
}
fn respond(request: Request, code: u16, content_type: &str, bytes: Vec<u8>) {
    let response = Response::from_data(bytes).with_status_code(StatusCode(code))
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
    let _ = request.respond(response);
}
pub fn run(config: Config) -> io::Result<()> {
    if !config.bind.ip().is_loopback()
        || !config.bind.is_ipv4()
        || !valid_host(&config.game_host)
        || !(1024..=65534).contains(&config.game_port)
    {
        return Err(bad(
            "Use a loopback IPv4 web bind and a valid game host/port.",
        ));
    }
    let authority = config.origin.strip_prefix("https://").or_else(|| {
        config
            .origin
            .strip_prefix("http://127.0.0.1:")
            .and_then(|port| {
                port.parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .map(|_| "localhost")
            })
    });
    if authority.is_none_or(|name| {
        name.is_empty()
            || name.contains(['/', '@', '?', '#'])
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
    }) {
        return Err(bad(
            "A canonical HTTPS public origin is required (local development may use http://127.0.0.1:PORT).",
        ));
    }
    let mut accounts = Accounts::open(&config.auth_data)?;
    let server = Server::http(config.bind).map_err(|_| bad("Cannot bind account web backend."))?;
    let mut limits = RateLimit::new(8);
    let mut global_start = auth::now()?;
    let mut global_count = 0u32;
    println!(
        "Account backend listening on {}; public origin {}. Passwords and tickets are never logged.",
        config.bind, config.origin
    );
    loop {
        let stop = config.auth_data.join("account-stop.request");
        if stop.exists() {
            std::fs::remove_file(stop)?;
            break;
        }
        let Some(mut request) = server.recv_timeout(std::time::Duration::from_millis(250))? else {
            continue;
        };
        let path = request.url().to_owned();
        if request.method() == &Method::Get && path == "/source" {
            let response = Response::empty(StatusCode(302))
                .with_header(header("Location", crate::licensing::SOURCE_URL))
                .with_header(header("Cache-Control", "no-store"));
            let _ = request.respond(response);
            continue;
        }
        if request.method() == &Method::Get {
            let (kind, bytes) = match path.as_str() {
                "/" => (
                    "text/html; charset=utf-8",
                    include_bytes!("../web/accounts/index.html").as_slice(),
                ),
                "/account.css" => (
                    "text/css; charset=utf-8",
                    include_bytes!("../web/accounts/account.css").as_slice(),
                ),
                "/account.js" => (
                    "text/javascript; charset=utf-8",
                    include_bytes!("../web/accounts/account.js").as_slice(),
                ),
                "/Start-Online.ps1" => (
                    "application/octet-stream",
                    include_bytes!("../../launcher/Start-Online.ps1").as_slice(),
                ),
                "/health" => ("application/json", b"{\"status\":\"ok\"}".as_slice()),
                _ => {
                    respond(
                        request,
                        404,
                        "application/json",
                        b"{\"message\":\"Not found.\"}".to_vec(),
                    );
                    continue;
                }
            };
            respond(request, 200, kind, bytes.to_vec());
            continue;
        }
        let (code, body) = handle(
            &mut request,
            &config,
            &mut accounts,
            &mut limits,
            &mut global_start,
            &mut global_count,
        )
        .unwrap_or((
            503,
            json!({"message":"The account service is temporarily unavailable."}),
        ));
        respond(
            request,
            code,
            "application/json; charset=utf-8",
            body.to_string().into_bytes(),
        );
    }
    Ok(())
}
fn handle(
    request: &mut Request,
    config: &Config,
    accounts: &mut Accounts,
    limits: &mut RateLimit,
    start: &mut u64,
    count: &mut u32,
) -> io::Result<(u16, Value)> {
    let reply = |code, message| Ok((code, json!({"message":message})));
    if request.method() != &Method::Post || !matches!(request.url(), "/api/register" | "/api/login")
    {
        return reply(404, "Not found.");
    }
    let time = auth::now()?;
    if time.saturating_sub(*start) >= 60 {
        *start = time;
        *count = 0;
    }
    let ip = match client_ip(request) {
        Ok(ip) => ip,
        Err(_) => return reply(400, "An IPv4 account connection is required."),
    };
    if *count >= 128 || !limits.allow(ip, time) {
        return reply(429, "Too many attempts. Please wait a minute.");
    }
    *count += 1;
    if request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Origin") && h.value.as_str() != config.origin)
    {
        return reply(403, "Use this server's account page.");
    }
    if !request.headers().iter().any(|h| {
        h.field.equiv("Content-Type")
            && h.value.as_str().split(';').next() == Some("application/json")
    }) {
        return reply(415, "Send a JSON request.");
    }
    if request.body_length().is_none_or(|length| length > 4096) {
        return reply(413, "Request is too large or has no declared length.");
    }
    let mut bytes = Vec::new();
    request.as_reader().take(4097).read_to_end(&mut bytes)?;
    let parsed = serde_json::from_slice::<Credentials>(&bytes);
    bytes.zeroize();
    let credentials = match parsed {
        Ok(value) => value,
        Err(_) => return reply(400, "Check the account and password fields."),
    };
    let account = credentials.account.trim().to_ascii_lowercase();
    if request.url() == "/api/register" {
        if reserved(&config.game_data, &account)? {
            return reply(409, "That account name is unavailable.");
        }
        match accounts.create(&account, &credentials.password) {
            Ok(()) => reply(
                201,
                "Account created. Sign in through the online launcher to enter the world.",
            ),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::AlreadyExists
                ) =>
            {
                reply(
                    400,
                    "Use an available account name (letters, digits or underscores) and a password of 12-128 characters.",
                )
            }
            Err(error) => Err(error),
        }
    } else {
        if !accounts.verify(&account, &credentials.password) {
            return reply(401, "Account or password is incorrect.");
        }
        let ticket = accounts.issue(&account, ip)?;
        Ok((
            200,
            json!({"account":account,"ticket":ticket,"expires_in":auth::TICKET_SECONDS,"game_host":config.game_host,"game_port":config.game_port}),
        ))
    }
}
