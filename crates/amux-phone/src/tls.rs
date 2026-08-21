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
//! The issuer is deliberately hobbled: a name constraint limits it to the
//! company's 10.0.0.0/8 range, so a phone that trusts it has not handed this
//! machine the power to vouch for anything else on the internet.

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
        tracing::info!("발급자를 새로 만들었습니다: {}", cert_path.display());

        Ok(Self { cert_pem, key_pem })
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
        params
            .distinguished_name
            .push(DnType::CommonName, ip.to_string());
        // Modern browsers ignore the common name entirely and read this.
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
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![
            GeneralSubtree::IpAddress(CidrSubnet::V4([10, 0, 0, 0], [255, 0, 0, 0])),
            // Loopback so the same issuer works when testing on the machine
            // itself. A phone's own 127.0.0.1 is the phone; nothing is opened up.
            GeneralSubtree::IpAddress(CidrSubnet::V4([127, 0, 0, 0], [255, 0, 0, 0])),
        ],
        excluded_subtrees: vec![],
    });
    params
}

pub fn default_ca_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("amux").join("ca")
}

fn write_private(dir: &Path, path: &Path, bytes: &[u8], mode: u32) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
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
