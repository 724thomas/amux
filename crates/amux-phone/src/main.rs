//! `amux-phone` — a sidecar that lets a phone browser drive a running amux.
//!
//! It changes nothing inside amux. It connects to the same local socket the
//! `amux` CLI uses, so it can be started and stopped freely without rebuilding
//! or restarting the app you are working in.
//!
//! Binds to loopback unless told otherwise: the token gate is proven from the
//! PC's own browser first, and only then is the door moved onto the network.

mod api;
mod auth;
mod rpc;

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use qrcode::render::unicode;
use qrcode::QrCode;

use crate::api::AppState;
use crate::auth::Auth;
use crate::rpc::Rpc;

#[derive(Parser)]
#[command(name = "amux-phone", version, about = "Serve a running amux to your phone's browser")]
struct Cli {
    /// Where to listen. Loopback by default; use 0.0.0.0:8000 to expose it to
    /// the network you are on — and know which network that is.
    #[arg(long, default_value = "127.0.0.1:8000")]
    bind: SocketAddr,

    /// amux socket name/path (defaults to $AMUX_SOCKET, then the per-user default)
    #[arg(long)]
    socket: Option<String>,

    /// Show a pairing code and QR even when a device is already registered.
    #[arg(long)]
    pair: bool,

    /// Required to bind anywhere but loopback. There is no TLS here: the
    /// device token is a shell on this machine and it travels in the clear on
    /// every poll, so opening this port means trusting the whole network it is
    /// opened onto. Say so out loud.
    #[arg(long)]
    insecure_plaintext: bool,

    /// Print registered devices and exit.
    #[arg(long)]
    devices: bool,

    /// Remove a device by the short id shown by --devices, then exit.
    #[arg(long, value_name = "ID")]
    revoke: Option<String>,

    /// Print a QR of the address (no pairing code) and keep going.
    ///
    /// A phone that is already paired only needs the address, and the address
    /// moves when the DHCP lease does. Re-scanning is faster than retyping an
    /// IP into a phone browser.
    #[arg(long)]
    qr: bool,

    /// Shut down after this many minutes with no request (0 disables).
    ///
    /// This is a door to a shell, and the usual session is "start it, walk to
    /// lunch, come back" — so forgetting to stop it is the normal mistake, not
    /// an unusual one. Closing itself is the default for that reason.
    #[arg(long, value_name = "MINUTES", default_value = "30")]
    idle_timeout: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "amux_phone=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let auth = Arc::new(Auth::load(config_path())?);

    if cli.devices {
        if auth.device_count() > 0 {
            println!("id        기기                  등록 (UTC)      마지막 사용 (UTC)  주소");
        }
        for d in auth.devices() {
            println!(
                "{}  {:<20}  {:<14}  {:<17}  {}",
                d.id,
                d.label,
                stamp(d.added),
                d.last_seen.map(stamp).unwrap_or_else(|| "-".into()),
                d.last_ip.unwrap_or_default(),
            );
        }
        if auth.device_count() == 0 {
            println!("등록된 기기가 없습니다.");
        }
        return Ok(());
    }
    if let Some(id) = cli.revoke {
        println!("{}", if auth.revoke(&id)? { "해제했습니다." } else { "그런 기기가 없습니다." });
        return Ok(());
    }

    // Binding past loopback hands a shell to everything on that network, over
    // plaintext. Refuse to do it by accident.
    if !cli.bind.ip().is_loopback() && !cli.insecure_plaintext {
        anyhow::bail!(
            "{} 은 loopback 이 아닙니다.\n\
             이 포트를 열면 그 네트워크의 모든 기기가 이 PC 의 셸에 닿을 수 있고, 토큰은 암호화 없이 오갑니다.\n\
             먼저 --bind 127.0.0.1:8000 으로 페어링을 끝낸 뒤, 알고서 여는 것이라면 --insecure-plaintext 를 붙이세요.",
            cli.bind
        );
    }

    let socket = cli.socket.unwrap_or_else(amux_protocol::default_socket_name);
    let rpc = Rpc::new(socket);

    // Fail loudly at startup rather than handing the phone a mystery later.
    match rpc.call("version", serde_json::Value::Null).await {
        Ok(v) => tracing::info!(
            "amux {} 에 연결했습니다 ({})",
            v.get("version").and_then(|x| x.as_str()).unwrap_or("?"),
            rpc.socket()
        ),
        Err(e) => tracing::warn!("amux 에 아직 연결하지 못했습니다 — {e}. 앱이 뜨면 이어집니다."),
    }

    let last_seen = Arc::new(std::sync::atomic::AtomicU64::new(api::now_secs()));
    let state = AppState { rpc, auth: auth.clone(), last_seen: last_seen.clone() };
    let app = api::router(state);

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    let shown = display_addr(cli.bind);
    tracing::info!("폰에서 열 주소: http://{shown}");
    if cli.bind.ip().is_loopback() {
        tracing::info!("지금은 이 PC 에서만 보입니다. 네트워크에 열려면 --bind 0.0.0.0:8000");
    }

    if !cli.bind.ip().is_loopback() {
        tracing::warn!("이 포트는 지금 네트워크에 열려 있고 통신은 평문입니다 — 신뢰하는 망에서만 쓰세요.");
    }

    if cli.qr {
        print_qr(&format!("http://{shown}"), None);
    }

    if cli.pair || auth.device_count() == 0 {
        tokio::spawn(pairing_loop(auth.clone(), shown.clone()));
    }

    if cli.idle_timeout > 0 {
        tracing::info!("{}분 동안 요청이 없으면 스스로 종료합니다 (--idle-timeout 0 으로 끔)", cli.idle_timeout);
        tokio::spawn(idle_shutdown(last_seen, cli.idle_timeout));
    }

    // ConnectInfo lets the token gate record which address used a device, so
    // an enrolment the user did not perform leaves a trace.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// `1787211417` → `08-20 16:36`. A device list is read to answer "is that one
/// mine, and when did it last call" — an epoch number cannot answer either.
/// Civil date from days-since-epoch, so no date crate is pulled in for this.
/// Show a URL as a scannable block, with the pairing code spelled out beneath
/// it when there is one — a camera may fail where six digits typed by hand will
/// not.
fn print_qr(url: &str, code: Option<&str>) {
    let caption = match code {
        Some(c) => format!("  페어링 코드 {c}  (60초)\n  {url}"),
        None => format!("  {url}"),
    };
    match QrCode::new(url.as_bytes()) {
        Ok(qr) => println!(
            "\n{}\n{caption}\n",
            qr.render::<unicode::Dense1x2>()
                .quiet_zone(true)
                .dark_color(unicode::Dense1x2::Light)
                .light_color(unicode::Dense1x2::Dark)
                .build()
        ),
        Err(e) => println!("\n{caption}\n  (QR 생성 실패: {e})\n"),
    }
}

/// Close the door when nobody has come through it for a while.
async fn idle_shutdown(last_seen: Arc<std::sync::atomic::AtomicU64>, minutes: u64) {
    let limit = minutes * 60;
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let idle = api::now_secs().saturating_sub(last_seen.load(std::sync::atomic::Ordering::Relaxed));
        if idle >= limit {
            println!("\n{minutes}분 동안 아무 요청이 없어 종료합니다. 다시 필요하면 같은 명령으로 띄우세요.\n");
            std::process::exit(0);
        }
    }
}

fn stamp(unix: u64) -> String {
    if unix == 0 {
        return "-".into();
    }
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;

    let z = days + 719_468;                       // shift the epoch to 0000-03-01
    let era = z.div_euclid(146_097);              // 400-year block
    let doe = z.rem_euclid(146_097);              // day within that block
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;                 // month counted from March
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let _y = if m <= 2 { y + 1 } else { y };      // calendar year (unused in the short form)

    format!("{:02}-{:02} {:02}:{:02}", m, d, secs / 3600, (secs % 3600) / 60)
}

/// Show a fresh six-digit code (and a QR carrying it) once a minute until a
/// device registers or ten minutes pass. The code only ever appears here, on
/// the machine's own screen — that is what keeps enrolment physical.
async fn pairing_loop(auth: Arc<Auth>, shown: String) {
    // The code is printed to this process's terminal. If that terminal is an
    // amux pane, every other agent in the app can read it back with
    // `pane.read_screen` — the screen is not a private channel here.
    if std::env::var_os(amux_protocol::env_keys::PANE_ID).is_some() {
        println!(
            "\n  주의: amux pane 안에서 실행 중입니다. 아래 페어링 코드는 다른 pane 의\n\
             \x20 에이전트가 pane.read_screen 으로 읽을 수 있습니다. 신경 쓰이면 amux 밖\n\
             \x20 터미널에서 실행하세요.\n"
        );
    }

    let already = auth.device_count();
    for _ in 0..10 {
        if auth.device_count() > already {
            println!("\n기기가 등록되었습니다. 페어링 코드를 끕니다.\n");
            return;
        }
        let code = auth.issue_pairing_code();
        print_qr(&format!("http://{shown}/?code={code}"), Some(&code));
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    println!("\n페어링 시간이 끝났습니다. 다시 하려면 --pair 로 실행하세요.\n");
}

/// A QR pointing at 0.0.0.0 helps nobody, so swap an unspecified bind for the
/// address this machine actually reaches the network on.
fn display_addr(bind: SocketAddr) -> String {
    if bind.ip().is_unspecified() {
        if let Some(ip) = outbound_ip() {
            return format!("{ip}:{}", bind.port());
        }
    }
    bind.to_string()
}

/// Ask the routing table which local address would be used to reach the
/// outside world. UDP connect sends no packets — it only picks a route.
fn outbound_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:80").ok()?; // TEST-NET-1: reserved, never routed
    sock.local_addr().ok().map(|a| a.ip())
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("amux").join("phone.json")
}
