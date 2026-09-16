//! Device tokens and the one-time pairing code.
//!
//! A password would authenticate *a person from any device*; what the user
//! asked for is "only my phone", which is a property of the device. So the
//! password step exists once, as a short code shown on the desktop, and after
//! that the phone carries a token nothing else has.
//!
//! ## What this does not defend against
//!
//! Anything running as the same OS user. The token store below is an ordinary
//! file that account can read *and write*, so a process with that account's
//! privileges can append a token of its own choosing and skip pairing entirely.
//! Agents inside amux panes are exactly that: `Pane::spawn()` hands each one
//! `$AMUX_SOCKET`, and they share this user. Keeping pairing off the amux RPC
//! surface raises the cost of enrolling a device and keeps an accident from
//! becoming one, but it is not a boundary. The boundary is the OS account.
//! What is on offer here is detection — `last_seen` and `last_ip` below — not
//! prevention.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How long a freshly shown pairing code stays usable.
const PAIRING_TTL: Duration = Duration::from_secs(60);

/// Wrong guesses a single code tolerates before it is burned.
///
/// Burning on the first wrong guess would make brute force impossible, but it
/// also hands anyone who can reach `/api/pair` a way to destroy every code the
/// moment it is issued — the user would watch their own correct code be
/// rejected forever with nothing on screen to explain it. Five guesses against
/// a six-digit code is still 5-in-a-million per code, which is not a way in.
const PAIRING_MAX_ATTEMPTS: u32 = 5;

/// Don't rewrite the token file on every single request just to move a
/// timestamp; once a minute is plenty for "when did this device last call".
const TOUCH_PERSIST_SECS: u64 = 60;

/// A label is printed to a terminal by `--devices`, so it must not be able to
/// forge rows or hide the id next to it.
const LABEL_MAX: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// The secret the phone sends on every request.
    pub token: String,
    pub label: String,
    /// Unix seconds, so the desktop can show "등록: 08-20".
    pub added: u64,
    /// Last time this token was accepted, and from where. A device the user
    /// never enrolled shows up here the moment it is used.
    #[serde(default)]
    pub last_seen: Option<u64>,
    #[serde(default)]
    pub last_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub label: String,
    pub added: u64,
    pub last_seen: Option<u64>,
    pub last_ip: Option<String>,
    /// First 8 characters — enough to tell two devices apart in a list without
    /// putting a usable secret on screen.
    pub id: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    devices: Vec<Device>,
}

struct Pairing {
    code: String,
    issued: Instant,
    attempts: u32,
}

pub struct Auth {
    path: PathBuf,
    stored: Mutex<Stored>,
    pairing: Mutex<Option<Pairing>>,
}

impl Auth {
    /// Read the device list, tolerating a missing file (first run).
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let stored = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("{} is unreadable ({e}); starting with no devices", path.display());
                Stored::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(anyhow::Error::from(e)),
        };
        Ok(Self {
            path,
            stored: Mutex::new(stored),
            pairing: Mutex::new(None),
        })
    }

    pub fn device_count(&self) -> usize {
        self.stored.lock().expect("auth lock").devices.len()
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.stored
            .lock()
            .expect("auth lock")
            .devices
            .iter()
            .map(|d| DeviceInfo {
                label: sanitize_label(&d.label),
                added: d.added,
                last_seen: d.last_seen,
                last_ip: d.last_ip.as_deref().map(sanitize_label),
                id: d.token.chars().take(8).collect(),
            })
            .collect()
    }

    /// Returns the device label when the token matches a registered device,
    /// and records that it was used so an unfamiliar device becomes visible.
    pub fn verify(&self, token: &str, from: Option<&str>) -> Option<String> {
        let mut stored = self.stored.lock().expect("auth lock");
        let now = unix_now();
        let idx = stored
            .devices
            .iter()
            .position(|d| constant_time_eq(d.token.as_bytes(), token.as_bytes()))?;

        let device = &mut stored.devices[idx];
        let label = device.label.clone();
        let stale = device.last_seen.map_or(true, |t| now.saturating_sub(t) >= TOUCH_PERSIST_SECS);
        device.last_seen = Some(now);
        if let Some(ip) = from {
            device.last_ip = Some(ip.to_string());
        }
        if stale {
            if let Err(e) = write_private(&self.path, &stored) {
                tracing::warn!("could not record last_seen: {e:#}");
            }
        }
        Some(label)
    }

    /// Mint a fresh six-digit code and start its 60-second window. Showing a
    /// new code invalidates the previous one.
    pub fn issue_pairing_code(&self) -> String {
        let code = six_digits();
        *self.pairing.lock().expect("pairing lock") = Some(Pairing {
            code: code.clone(),
            issued: Instant::now(),
            attempts: 0,
        });
        code
    }

    /// Trade a live code for a device token.
    pub fn pair(&self, code: &str, label: &str, from: Option<&str>) -> anyhow::Result<Option<Device>> {
        let matched = {
            let mut slot = self.pairing.lock().expect("pairing lock");
            match slot.as_mut() {
                Some(p) if p.issued.elapsed() > PAIRING_TTL => {
                    *slot = None;
                    false
                }
                Some(p) => {
                    if constant_time_eq(p.code.as_bytes(), code.trim().as_bytes()) {
                        *slot = None; // consumed by success
                        true
                    } else {
                        p.attempts += 1;
                        if p.attempts >= PAIRING_MAX_ATTEMPTS {
                            tracing::warn!("페어링 코드가 {PAIRING_MAX_ATTEMPTS}회 틀려 무효화했습니다");
                            *slot = None;
                        }
                        false
                    }
                }
                None => false,
            }
        };
        if !matched {
            return Ok(None);
        }

        let device = Device {
            token: new_token(),
            label: {
                let clean = sanitize_label(label);
                if clean.is_empty() { "내 폰".into() } else { clean }
            },
            added: unix_now(),
            last_seen: None,
            last_ip: from.map(str::to_string),
        };
        {
            let mut stored = self.stored.lock().expect("auth lock");
            stored.devices.push(device.clone());
            write_private(&self.path, &stored)?;
        }
        Ok(Some(device))
    }

    /// Remove a device by the short id shown in the list. Losing the phone
    /// should cost one line, not a password change.
    ///
    /// The id must be at least as long as what `devices()` prints: a prefix
    /// match on a shorter string — an empty one above all — would quietly take
    /// every device with it.
    pub fn revoke(&self, id: &str) -> anyhow::Result<bool> {
        let id = id.trim();
        if id.len() < 8 {
            anyhow::bail!("기기 id 는 8자 이상이어야 합니다 (--devices 가 보여 주는 그대로)");
        }
        let mut stored = self.stored.lock().expect("auth lock");
        let before = stored.devices.len();
        stored.devices.retain(|d| !d.token.starts_with(id));
        if stored.devices.len() == before {
            return Ok(false);
        }
        write_private(&self.path, &stored)?;
        Ok(true)
    }
}

/// 244 bits of randomness: two v4 UUIDs give 122 usable bits each.
fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn six_digits() -> String {
    let bytes = Uuid::new_v4().into_bytes();
    let n = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
    format!("{n:06}")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Strip anything that could move the cursor or start a new row when this text
/// is printed to a terminal, and cap the length. Without this a device could
/// name itself "\r\x1b[K…" and erase the id printed beside it, making itself
/// unrevokable.
fn sanitize_label(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(LABEL_MAX)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Length-independent comparison, so a mismatch never leaks where it happened.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Write the token file so only this account can read it, and never let it
/// exist at wider permissions even briefly — the mode is set as the file is
/// created, not fixed up afterwards.
fn write_private(path: &Path, stored: &Stored) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let json = serde_json::to_vec_pretty(stored)?;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, &json)?;

    std::fs::rename(&tmp, path)?;
    Ok(())
}
