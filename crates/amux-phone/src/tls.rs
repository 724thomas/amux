//! A certificate authority of one, kept on this machine.
//!
//! The problem this solves is not "the phone shows a warning" — a single
//! self-signed certificate would silence that too. It is that a certificate
//! names an address, and this laptop's address moves with its DHCP lease. A
//! certificate pinned to one IP has to be re-issued *and re-trusted on the
//! phone* every time that happens, which is a chore nobody keeps up.
//!
//! Browsers trust the *issuer*, not the leaf. So the issuer is created once and
//! installed on the phone once; after that a fresh leaf is minted for whatever
//! address we hold at startup, and the phone accepts it without being touched
//! again.
//!
//! The issuer is deliberately hobbled: a name constraint limits it to private
//! address space — the ranges a LAN can actually use — so a phone that trusts
//! it has not handed this machine the power to vouch for anything else on the
//! internet. Domain names are excluded outright, so the widest thing a stolen
//! key could impersonate is a device on a private network, never a site.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DnType, GeneralSubtree, IsCa, Issuer,
    KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};

/// Leaf lifetime. Short is free here — a new one is issued on every start —
/// and Apple rejects long-lived leaves outright.
const LEAF_DAYS: i64 = 60;
/// The issuer has to outlive many leaves, or the phone needs re-enrolling.
const CA_YEARS: i64 = 10;

/// Bumped whenever `ca_params()` changes what the issuer will vouch for.
///
/// An issuer already on disk is reused as-is, and it has to be: the phone
/// trusts that exact certificate, so quietly replacing it would break every
/// paired device. But that also means a change to `ca_params()` reaches only
/// issuers created afterwards. Recording the schema an issuer was built with
/// is what lets us notice, and say so, instead of letting an old one look
/// current.
///
/// - **v2** added the blanket DNS exclusion; a v1 issuer can sign a
///   certificate for any domain name.
/// - **v3** widened the permitted addresses from 10.0.0.0/8 to all of private
///   address space; a v2 issuer cannot vouch for a home or small-office
///   router's 192.168.x or 172.16–31.x range.
const CA_SCHEMA: u32 = 3;

/// The schema of the issuer sitting in `dir`, or the current one when there is
/// no issuer yet — the next one created will be built to today's rules.
fn schema_on_disk(dir: &Path) -> u32 {
    if !dir.join("ca.crt").exists() {
        return CA_SCHEMA;
    }
    std::fs::read_to_string(dir.join("ca.schema"))
        .ok()
        .and_then(|v| v.trim().parse().ok())
        // An issuer with no schema file predates the marker: that is v1.
        .unwrap_or(1)
}

pub struct Ca {
    pub cert_pem: String,
    key_pem: String,
}

impl Ca {
    /// Read the issuer from disk if it is already there, without creating one.
    /// Used so a plain-HTTP run can still hand the certificate to a phone.
    pub fn load_if_exists(dir: &Path) -> anyhow::Result<Option<Self>> {
        let cert_path = dir.join("ca.crt");
        let key_path = dir.join("ca.key");
        if !cert_path.exists() || !key_path.exists() {
            return Ok(None);
        }
        Ok(Some(Self {
            cert_pem: std::fs::read_to_string(&cert_path)?,
            key_pem: std::fs::read_to_string(&key_path)?,
        }))
    }

    /// Read the issuer from disk, creating it the first time.
    pub fn load_or_create(dir: &Path) -> anyhow::Result<Self> {
        let cert_path = dir.join("ca.crt");
        let key_path = dir.join("ca.key");

        if cert_path.exists() && key_path.exists() {
            warn_if_outdated(dir);
            return Ok(Self {
                cert_pem: std::fs::read_to_string(&cert_path)?,
                key_pem: std::fs::read_to_string(&key_path)?,
            });
        }

        let key = KeyPair::generate().context("generating the issuer key")?;
        let mut params = ca_params();
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(365 * CA_YEARS);

        let cert = params.self_signed(&key).context("signing the issuer")?;
        let cert_pem = cert.pem();
        let key_pem = key.serialize_pem();

        write_private(dir, &cert_path, cert_pem.as_bytes(), 0o644)?;
        write_private(dir, &key_path, key_pem.as_bytes(), 0o600)?;
        let _ = std::fs::write(dir.join("ca.schema"), CA_SCHEMA.to_string());
        tracing::info!("발급자를 새로 만들었습니다: {}", cert_path.display());

        Ok(Self { cert_pem, key_pem })
    }

    /// SHA-256 over the issuer's DER, formatted the way install screens show it.
    ///
    /// The issuer is handed to the phone over plaintext HTTP, which protects it
    /// from being read but not from being swapped. A swapped issuer is worse
    /// than a stolen token: the phone would trust someone else's authority for
    /// ten years. Comparing this string against what the phone displays before
    /// tapping install is what moves that step off the network and onto two
    /// screens the user can see at once.
    pub fn fingerprint(&self) -> String {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};

        let body: String = self
            .cert_pem
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .unwrap_or_default();
        Sha256::digest(&der)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .chunks(8)
            .map(|c| c.join(" "))
            .collect::<Vec<_>>()
            .join("\n                 ")
    }

    /// Mint a certificate for the address we are about to listen on.
    pub fn issue_for(&self, ip: IpAddr) -> anyhow::Result<(String, String)> {
        // The issuer is rebuilt from the same description used to create it,
        // paired with the key read from disk. Nothing is parsed back out of the
        // certificate: the name the leaf points at comes from this description,
        // so as long as both sides call `ca_params()` the chain lines up.
        let issuer_key = KeyPair::from_pem(&self.key_pem).context("reading the issuer key")?;
        let issuer = Issuer::new(ca_params(), issuer_key);

        let key = KeyPair::generate().context("generating the server key")?;
        let mut params = CertificateParams::default();
        // Spelled out rather than left to the library's default. Verifiers read
        // the subject alternative name below and ignore the common name, but
        // OpenSSL still treats a common name as a candidate host name when
        // matching DNS constraints - putting the address here made the issuer's
        // blanket DNS exclusion reject its own server certificates. The spaces
        // are what keep this from parsing as a host name, so it must not be
        // inherited from elsewhere: if it silently became host-shaped, every
        // certificate this issuer signs would start failing.
        params
            .distinguished_name
            .push(DnType::CommonName, "amux phone server");
        params.subject_alt_names = vec![SanType::IpAddress(ip)];
        params.use_authority_key_identifier_extension = true;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(LEAF_DAYS);

        let cert = params
            .signed_by(&key, &issuer)
            .context("signing the server certificate")?;

        // The phone is given the leaf and the issuer together, so a browser
        // that has the issuer installed can build the chain without guessing.
        Ok((
            format!("{}{}", cert.pem(), self.cert_pem),
            key.serialize_pem(),
        ))
    }
}

/// An issuer built before a change keeps working and keeps being trusted,
/// which is exactly why it must not pass unmentioned.
fn warn_if_outdated(dir: &Path) {
    let found = schema_on_disk(dir);
    if found >= CA_SCHEMA {
        return;
    }
    // Two different problems, and conflating them would misinform: v1 is a
    // safety hole, v2 is merely narrow. Both are fixed the same way, so the
    // instructions are shared and only the diagnosis differs.
    let problem = if found < 2 {
        "이 발급자는 IP 만 제한하고 도메인 이름은 제한하지 않습니다. 즉 이 발급자를\n\
         \x20 신뢰하는 기기에 대해서는 임의의 도메인을 사칭하는 인증서를 만들 수 있습니다."
    } else {
        "이 발급자는 10.x 주소만 보증합니다. 지금은 192.168.x 와 172.16~31.x 도 보증\n\
         \x20 범위에 들어가지만, 이미 만들어진 발급자는 그대로라 그런 주소로 찍은 인증서는\n\
         \x20 폰이 거부합니다. (10.x 나 127.x 로 바인딩하는 동안은 아무 문제 없습니다.)"
    };
    let script = if cfg!(windows) { "scripts\\phone.ps1" } else { "scripts/phone.sh" };
    println!(
        "\n  ── 발급자가 오래된 형식입니다 (v{found}, 지금은 v{CA_SCHEMA}) ─────────────\n\
         \x20 {problem}\n\
         \x20 코드는 고쳐졌지만 이미 만들어진 발급자 파일은 바뀌지 않습니다.\n\
         \x20\n\
         \x20 고치려면 발급자를 새로 만들고 폰에 다시 설치해야 합니다.\n\
         \x20   1) {}/ca.crt 와 {}/ca.key 를 지우세요\n\
         \x20   2) {script} --setup   (새 발급자를 만들어 폰에 넘깁니다)\n\
         \x20   3) 폰에서 옛 발급자를 삭제한 뒤 새 것을 설치\n\
         \x20   4) {script}          (주소가 그대로면 기기 등록은 그대로 살아 있습니다)\n",
        dir.display(),
        dir.display()
    );
}

/// How the issuer describes itself. Used both when creating it and when
/// signing with it later.
fn ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "amux phone (local issuer)");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    // Vouching for the office range and nothing else: a phone that trusts this
    // issuer has not given this laptop authority over the rest of the internet.
    //
    // Both halves are needed. A constraint binds only the *name form* it names,
    // and a form left unmentioned stays unrestricted - so permitting IP ranges
    // alone would still let this issuer sign `bank.example.com`, because that is
    // a DNS name and no DNS rule was stated. The empty entries below exclude
    // every name of those forms, which is what closes that door.
    params.name_constraints = Some(NameConstraints {
        // Private address space, all three blocks of it (RFC 1918). The office
        // is 10.x, but a home or small-office router hands out 192.168.x and
        // some hand out 172.16–31.x; an issuer that cannot vouch for those
        // cannot be used from the networks people actually sit on. None of
        // these is routable on the internet, so the guarantee is unchanged:
        // whatever this issuer signs is only ever believed for a machine on a
        // local network.
        permitted_subtrees: vec![
            GeneralSubtree::IpAddress(CidrSubnet::V4([10, 0, 0, 0], [255, 0, 0, 0])),
            GeneralSubtree::IpAddress(CidrSubnet::V4([172, 16, 0, 0], [255, 240, 0, 0])),
            GeneralSubtree::IpAddress(CidrSubnet::V4([192, 168, 0, 0], [255, 255, 0, 0])),
            // Loopback so the same issuer works when testing on the machine
            // itself. A phone's own 127.0.0.1 is the phone; nothing is opened up.
            GeneralSubtree::IpAddress(CidrSubnet::V4([127, 0, 0, 0], [255, 0, 0, 0])),
        ],
        excluded_subtrees: vec![
            // An empty name matches every name of that form, so this excludes
            // DNS names outright. Only DNS is excluded: an empty rfc822Name
            // entry was tried and rejected every certificate this issuer signs,
            // including its own server ones, so it buys nothing here.
            GeneralSubtree::DnsName(String::new()),
        ],
    });
    params
}

/// Would the issuer in `dir` be willing to vouch for this address?
///
/// Signing happens whether or not the answer is yes - name constraints bind the
/// verifier, not the signer - so an address outside the permitted set yields a
/// certificate that every client rejects. On the machine running the server
/// nothing looks wrong; the failure appears only on the phone. Answering the
/// question before binding is what turns that into a startup error. IPv6 lands
/// here too: the permitted set is IPv4 only, so an IPv6 bind is out of range.
///
/// The permitted set belongs to the certificate that exists, not to this
/// build. Widening `ca_params` reaches only issuers created afterwards, so an
/// issuer written before v3 still refuses everything outside 10.x — and asking
/// the compiled-in ranges instead would wave exactly the address through that
/// the phone is about to reject.
pub fn covers(dir: &Path, ip: IpAddr) -> bool {
    let IpAddr::V4(v4) = ip else { return false };
    let o = v4.octets();
    if o[0] == 10 || o[0] == 127 {
        return true;
    }
    schema_on_disk(dir) >= 3
        && ((o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168))
}

/// Where the issuer is kept: the same per-user config directory the app uses
/// for its session file, so every amux tool on the machine agrees on one spot.
/// Resolving it anywhere else would put a private key in whatever directory
/// the server happened to start in — see `amux_protocol::config_dir`.
pub fn default_ca_dir() -> PathBuf {
    config_base().join("amux").join("ca")
}

pub(crate) fn config_base() -> PathBuf {
    amux_protocol::config_dir()
}

fn write_private(dir: &Path, path: &Path, bytes: &[u8], mode: u32) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    // Windows has no mode bits. What keeps the key private there is the ACL
    // NTFS already puts on the user profile `%APPDATA%` lives under, which
    // grants the owner and administrators and nobody else — the same standing
    // that protects `~/.ssh` once 0700 is set. There is nothing to apply here,
    // so the mode is Unix-only.
    let _ = mode;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of our own, removed when the test ends however it ends.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("amux-ca-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).expect("temp dir");
            Self(p)
        }
        /// Stand in for an issuer built by an older version: what `covers`
        /// reads is the pair of marker files, not the certificate itself.
        fn with_issuer(self, schema: Option<u32>) -> Self {
            std::fs::write(self.0.join("ca.crt"), "not a real certificate").unwrap();
            if let Some(v) = schema {
                std::fs::write(self.0.join("ca.schema"), v.to_string()).unwrap();
            }
            self
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// With no issuer yet, the next one will be built to today's rules, so
    /// every private range is fair game — and nothing outside them is.
    #[test]
    fn a_fresh_install_covers_private_space_and_nothing_else() {
        let dir = Dir::new("fresh");
        for addr in ["10.1.2.3", "172.16.0.9", "172.31.255.1", "192.168.219.100", "127.0.0.1"] {
            assert!(covers(dir.path(), ip(addr)), "{addr} should be covered");
        }
        for addr in ["8.8.8.8", "172.15.0.1", "172.32.0.1", "192.169.0.1", "203.0.113.7"] {
            assert!(!covers(dir.path(), ip(addr)), "{addr} must not be covered");
        }
        // The permitted set is IPv4 only, so an IPv6 bind is out of range.
        assert!(!covers(dir.path(), ip("::1")));
    }

    /// The trap this guard exists for: an issuer created before the ranges
    /// widened cannot vouch for 192.168.x, and the phone is the only place
    /// that would ever say so. Refusing at startup is the whole point.
    #[test]
    fn an_issuer_from_before_the_widening_still_only_covers_ten_dot() {
        for schema in [None, Some(1), Some(2)] {
            let dir = Dir::new(&format!("old{}", schema.unwrap_or(0))).with_issuer(schema);
            assert!(covers(dir.path(), ip("10.1.2.3")), "10.x, schema {schema:?}");
            assert!(covers(dir.path(), ip("127.0.0.1")), "loopback, schema {schema:?}");
            assert!(
                !covers(dir.path(), ip("192.168.219.100")),
                "192.168.x must be refused for schema {schema:?}",
            );
            assert!(!covers(dir.path(), ip("172.16.0.9")), "172.16.x, schema {schema:?}");
        }
    }

    /// An issuer recorded at the current schema was built with the wide set.
    #[test]
    fn a_current_issuer_covers_the_wide_set() {
        let dir = Dir::new("current").with_issuer(Some(CA_SCHEMA));
        assert!(covers(dir.path(), ip("192.168.219.100")));
        assert!(covers(dir.path(), ip("172.20.1.1")));
    }

    /// End to end: a real issuer is created, stamped with the schema, and
    /// mints a leaf for a home-router address. Signing succeeds regardless of
    /// the name constraints, so what this actually pins down is that rcgen
    /// accepts the widened `ca_params` and that the marker is written.
    #[test]
    fn a_created_issuer_is_stamped_and_signs_for_a_home_address() {
        let dir = Dir::new("issue");
        let ca = Ca::load_or_create(dir.path()).expect("create issuer");
        assert_eq!(schema_on_disk(dir.path()), CA_SCHEMA, "schema marker");

        let (chain, key) = ca.issue_for(ip("192.168.219.100")).expect("issue leaf");
        assert!(chain.contains("BEGIN CERTIFICATE"), "leaf + issuer chain");
        assert!(key.contains("PRIVATE KEY"), "leaf key");

        // Reloading must reuse the same issuer: the phone trusts that exact
        // certificate, so a second run may not quietly mint a different one.
        let again = Ca::load_or_create(dir.path()).expect("reload issuer");
        assert_eq!(again.fingerprint(), ca.fingerprint(), "issuer was replaced");
    }
}
