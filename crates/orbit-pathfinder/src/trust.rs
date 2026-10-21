//! Certificates the user chose to trust, for addresses on the local network
//! only (a Proxmox, a router, a NAS: self-signed, from no known authority).
//! A certificate WebKit refuses is otherwise refused outright; a public
//! site is never offered this.
//!
//! Each exception is a host and the certificate it showed, kept in
//! `trusted-certificates.json` (readable by its owner only) and given to
//! every profile's session as it is made. A host that later shows another
//! certificate is asked about again — and told that it changed.

use std::path::{Path, PathBuf};

use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trusted {
    pub host: String,
    /// The certificate, PEM.
    pub pem: String,
    /// Its SHA-256 fingerprint, for the eye.
    pub sha256: String,
}

fn file(data: &Path) -> PathBuf {
    data.join("trusted-certificates.json")
}

pub fn load(data: &Path) -> Vec<Trusted> {
    std::fs::read_to_string(file(data)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Keeps `certificate` as trusted for `host`, in place of any it had.
pub fn add(data: &Path, host: &str, certificate: &gio::TlsCertificate) {
    let Some(pem) = certificate.certificate_pem() else { return };
    let mut all = load(data);
    all.retain(|t| t.host != host);
    all.push(Trusted { host: host.to_owned(), pem: pem.to_string(), sha256: fingerprint(certificate) });
    if let Err(err) = write_private(&file(data), &all) {
        log::warn!("trusted certificates: {err}");
    }
}

/// Whether `host` had another certificate trusted than `certificate`.
pub fn changed(data: &Path, host: &str, certificate: &gio::TlsCertificate) -> bool {
    let now = fingerprint(certificate);
    load(data).iter().any(|t| t.host == host && t.sha256 != now)
}

/// Gives `session` every trusted certificate.
pub fn apply(data: &Path, session: &webkit6::NetworkSession) {
    for t in load(data) {
        match gio::TlsCertificate::from_pem(&t.pem) {
            Ok(certificate) => session.allow_tls_certificate_for_host(&certificate, &t.host),
            Err(err) => log::warn!("trusted certificate for {}: {err}", t.host),
        }
    }
}

/// "AB:CD:…" — SHA-256 of the certificate's DER.
pub fn fingerprint(certificate: &gio::TlsCertificate) -> String {
    let Some(der) = certificate.certificate() else { return String::new() };
    let hex = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &der).map(|s| s.to_uppercase()).unwrap_or_default();
    hex.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(":")
}

fn write_private(file: &Path, all: &[Trusted]) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(all).map_err(std::io::Error::other)?;
    let tmp = file.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(text.as_bytes())?;
    std::fs::rename(&tmp, file)
}
