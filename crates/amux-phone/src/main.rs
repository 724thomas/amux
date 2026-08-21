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
mod tls;

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
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

    /// Serve over HTTPS with a certificate issued by this machine's own local
    /// issuer, created on first use.
    ///
    /// Without this the device token — which is a shell on this machine —
    /// crosses the company network in the clear on every poll.
    #[arg(long)]
    tls: bool,

    /// Serve over plain HTTP for one session so the phone can fetch and install
    /// the local issuer. Creates the issuer if it does not exist yet.
    ///
    /// This step cannot itself be over HTTPS: the phone has no way to trust the
    /// server until it has the very file it is here to collect.
    #[arg(long, conflicts_with = "tls")]
    setup_ca: bool,

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
    // plaintext. Refuse to do it by accident. With --tls there is no plaintext
    // to object to, so the flag is not asked for.
    if !cli.bind.ip().is_loopback() && !cli.tls && !cli.setup_ca && !cli.insecure_plaintext {
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
    // Issue a certificate for the address we are about to answer on. A fresh
    // leaf every start is what lets the DHCP lease move without the phone ever
    // being touched again.
    // A certificate has to name an address, and 0.0.0.0 is not one - it means
    // "every interface". Issuing for it produces a certificate no client can
    // match, which shows up on the phone as an error while the address printed
    // here looks perfectly right.
    if cli.tls && cli.bind.ip().is_unspecified() {
        anyhow::bail!(
            "--tls 는 구체적인 주소가 필요합니다 ({} 은 \"모든 인터페이스\"라는 뜻이라 인증서가 보증할 이름이 없습니다).\n\
             scripts/phone.sh 를 쓰면 현재 주소를 알아서 찾아 넘깁니다.",
            cli.bind.ip()
        );
    }

    // 서명은 어차피 되지만, 발급자가 보증하지 못하는 이름이면 모든 클라이언트가
    // 거부합니다. 이 PC 에서는 멀쩡해 보이고 폰에서만 오류가 나는 형태라, 뜨기
    // 전에 막습니다. IPv6 도 여기서 걸립니다 - 허용 범위가 IPv4 뿐입니다.
    if cli.tls && !tls::covers(cli.bind.ip()) {
        anyhow::bail!(
            "{} 은 이 발급자가 보증할 수 있는 범위 밖입니다.\n\
             발급자는 10.0.0.0/8 과 127.0.0.0/8 만 보증하도록 제한돼 있어서, 그 밖의 주소로\n\
             인증서를 찍으면 폰이 거부합니다. 사내 주소나 loopback 으로 바인딩하세요.",
            cli.bind.ip()
        );
    }

    let mut ca_fingerprint: Option<String> = None;
    let ca_dir = tls::default_ca_dir();
    let (tls_config, ca_pem) = if cli.tls {
        let ca = tls::Ca::load_or_create(&ca_dir)?;
        let (chain, key) = ca.issue_for(cli.bind.ip())?;
        let config = axum_server::tls_rustls::RustlsConfig::from_pem(
            chain.into_bytes(),
            key.into_bytes(),
        )
        .await
        .context("loading the issued certificate")?;
        (Some(config), Some(Arc::new(ca.cert_pem)))
    } else if cli.setup_ca {
        let ca = tls::Ca::load_or_create(&ca_dir)?;
        ca_fingerprint = Some(ca.fingerprint());
        (None, Some(Arc::new(ca.cert_pem)))
    } else {
        // Already-created issuer is still served, so a plain run can hand it over.
        (None, tls::Ca::load_if_exists(&ca_dir)?.map(|ca| Arc::new(ca.cert_pem)))
    };

    let state = AppState {
        rpc,
        auth: auth.clone(),
        last_seen: last_seen.clone(),
        ca_pem,
    };
    // 설치 모드는 평문일 수밖에 없으므로, 인증이 필요한 경로를 아예 달지 않습니다.
    // 등록만 막는 것으로는 부족합니다 - 평문 시절에 받아 둔 토큰을 아직 들고 있는
    // 폰이 그 세션 동안 화면을 평문으로 계속 받아 갈 수 있기 때문입니다.
    let app = if cli.setup_ca {
        api::setup_router(state)
    } else {
        api::router(state)
    };

    let scheme = if cli.tls { "https" } else { "http" };
    let shown = display_addr(cli.bind);
    tracing::info!("폰에서 열 주소: {scheme}://{shown}");
    if cli.bind.ip().is_loopback() {
        tracing::info!("지금은 이 PC 에서만 보입니다. 네트워크에 열려면 --bind 0.0.0.0:8000");
    }

    if !cli.bind.ip().is_loopback() {
        tracing::warn!("이 포트는 지금 네트워크에 열려 있고 통신은 평문입니다 — 신뢰하는 망에서만 쓰세요.");
    }

    if cli.setup_ca {
        let url = format!("http://{shown}/ca.crt");
        println!(
            "\n  ── 발급자 설치 (딱 한 번) ──────────────────────────────\n\
             \x20 폰에서 아래 주소를 열어 내려받고 설치하세요.\n\
             \x20 아이폰: 설정 → 일반 → VPN 및 기기 관리 에서 설치한 뒤,\n\
             \x20         설정 → 일반 → 정보 → 인증서 신뢰 설정 에서 스위치를 켜야 합니다.\n\
             \x20 안드로이드: 설정 → 보안 → 인증서 설치 → CA 인증서\n\
             \x20\n\
             \x20 끝나면 Ctrl+C 하고 --tls 로 다시 띄우세요. 주소가 http 에서 https 로\n\
             \x20 바뀌므로 브라우저가 다른 사이트로 취급합니다 — 폰 등록을 한 번 더\n\
             \x20 해야 하니 그때는 --pair 를 붙이세요.\n"
        );
        print_qr(&url, None);
        if let Some(pem) = ca_fingerprint.as_deref() {
            println!(
                "  설치 화면에 뜨는 지문이 아래와 같은지 눈으로 대조하세요.\n\
                 \x20 다르면 중간에서 바꿔치기된 것이니 설치하지 마십시오.\n\n\
                 \x20 SHA-256  {pem}\n"
            );
        }
    }

    if cli.qr {
        print_qr(&format!("{scheme}://{shown}"), None);
    }

    // Not while handing over the issuer. That session is plaintext by
    // necessity, and pairing there would put the device token - which is a
    // shell on this machine - into a plaintext response body. Collect the
    // certificate first, then pair once the door is HTTPS.
    if (cli.pair || auth.device_count() == 0) && !cli.setup_ca {
        tokio::spawn(pairing_loop(auth.clone(), shown.clone(), scheme));
    }

    // 설치 모드는 평문이므로 오래 떠 있으면 안 됩니다. 안내를 읽고 폰에서
    // 설치하는 데 필요한 만큼만 두고 스스로 닫습니다.
    if cli.setup_ca {
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(15 * 60)).await;
            println!("\n설치 모드는 15분만 열려 있습니다. 종료합니다.\n");
            std::process::exit(0);
        });
    }

    if cli.idle_timeout > 0 {
        tracing::info!("{}분 동안 요청이 없으면 스스로 종료합니다 (--idle-timeout 0 으로 끔)", cli.idle_timeout);
        tokio::spawn(idle_shutdown(last_seen, cli.idle_timeout));
    }

    // ConnectInfo lets the token gate record which address used a device, so
    // an enrolment the user did not perform leaves a trace.
    let service = app.into_make_service_with_connect_info::<SocketAddr>();
    match tls_config {
        Some(config) => axum_server::bind_rustls(cli.bind, config).serve(service).await?,
        None => {
            let listener = tokio::net::TcpListener::bind(cli.bind).await?;
            axum::serve(listener, service).await?
        }
    }
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
async fn pairing_loop(auth: Arc<Auth>, shown: String, scheme: &'static str) {
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
        print_qr(&format!("{scheme}://{shown}/?code={code}"), Some(&code));
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
