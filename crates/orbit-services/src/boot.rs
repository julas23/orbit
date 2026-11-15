//! Going back to a snapshot (docs/PLANO-DISTRO.md §6.4): what the helper
//! and the snapper plugin need to know about the boot loader's UKIs, kept
//! apart from running anything so it can be tested.
//!
//! The root boots from btrfs's default subvolume; `snapper rollback`
//! changes it. The kernel and the initramfs live inside the root (`/boot`
//! is in `@`), so each snapshot holds its own, and after a rollback the
//! ESP's UKIs are made again from the kernels of the new default: a UKI
//! whose kernel the new root has no modules for would start a system
//! without its drivers. The system left behind gets one UKI of its own,
//! `orbit-previous.efi`, which starts it by its subvolume, so the way
//! back is in the boot menu.

use std::path::Path;

/// The UKI that starts the system from before the last rollback.
pub const PREVIOUS: &str = "previous";

/// The kernel version a UKI on the ESP is for, from its file name
/// (`TOKEN-VERSION.efi`, with boot counting `TOKEN-VERSION+3.efi` or
/// `TOKEN-VERSION+2-1.efi`); none for another file or for the previous
/// system's UKI.
pub fn uki_version(file: &str, token: &str) -> Option<String> {
    let rest = file.strip_prefix(token)?.strip_prefix('-')?.strip_suffix(".efi")?;
    let version = strip_counter(rest);
    (version != PREVIOUS && version.starts_with(|c: char| c.is_ascii_digit())).then(|| version.to_owned())
}

/// `VERSION+LEFT` or `VERSION+LEFT-DONE` without the counter.
fn strip_counter(name: &str) -> &str {
    let Some((version, counter)) = name.rsplit_once('+') else { return name };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let ok = match counter.split_once('-') {
        Some((left, done)) => digits(left) && digits(done),
        None => digits(counter),
    };
    if ok { version } else { name }
}

/// The kernels a root holds whole: its vmlinuz and initrd in /boot and its
/// modules, oldest first.
pub fn kernels(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join("boot"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("vmlinuz-").map(str::to_owned))
        .filter(|v| {
            root.join(format!("boot/initrd.img-{v}")).is_file() && root.join(format!("usr/lib/modules/{v}")).is_dir()
        })
        .collect();
    out.sort_by(|a, b| compare_versions(a, b));
    out
}

/// Kernel versions compared by their numbers, then as text.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let numbers = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect()
    };
    numbers(a).cmp(&numbers(b)).then_with(|| a.cmp(b))
}

/// The kernel command line made to start the root from `subvol` (as
/// findmnt's FSROOT gives it): `subvol=` first in `rootflags=`, any other
/// `subvol`/`subvolid` there dropped.
pub fn cmdline_with_subvol(cmdline: &str, subvol: &str) -> String {
    let mut found = false;
    let mut words: Vec<String> = cmdline
        .split_whitespace()
        .map(|w| match w.strip_prefix("rootflags=") {
            Some(flags) => {
                found = true;
                let mut list = vec![format!("subvol={subvol}")];
                list.extend(
                    flags
                        .split(',')
                        .filter(|f| !f.is_empty() && !f.starts_with("subvol=") && !f.starts_with("subvolid="))
                        .map(str::to_owned),
                );
                format!("rootflags={}", list.join(","))
            }
            None => w.to_owned(),
        })
        .collect();
    if !found {
        words.push(format!("rootflags=subvol={subvol}"));
    }
    words.join(" ")
}

/// os-release for the previous system's UKI: its name says what it is in
/// the boot menu, and its IMAGE_ID (systemd-boot's sort key) puts it after
/// the kernels of the system that starts now.
pub fn previous_os_release(os_release: &str, token: &str) -> String {
    let name = os_release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_owned())
        .unwrap_or_else(|| "Linux".into());
    let mut out: Vec<String> = os_release
        .lines()
        .filter(|l| !l.starts_with("PRETTY_NAME=") && !l.starts_with("IMAGE_ID="))
        .map(str::to_owned)
        .collect();
    out.push(format!("PRETTY_NAME=\"{name}, before going back to a snapshot\""));
    out.push(format!("IMAGE_ID={token}-{PREVIOUS}"));
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_file_names() {
        let v = |f| uki_version(f, "orbit");
        assert_eq!(v("orbit-6.12.117+deb13-amd64+3.efi").as_deref(), Some("6.12.117+deb13-amd64"));
        assert_eq!(v("orbit-6.12.117+deb13-amd64+2-1.efi").as_deref(), Some("6.12.117+deb13-amd64"));
        assert_eq!(v("orbit-6.12.117+deb13-amd64.efi").as_deref(), Some("6.12.117+deb13-amd64"));
        assert_eq!(v("orbit-previous.efi"), None);
        assert_eq!(v("debian-6.12.117.efi"), None);
        assert_eq!(v("orbit-6.12.117+deb13-amd64.conf"), None);
    }

    #[test]
    fn versions_in_order() {
        let mut list = vec!["6.12.117+deb13-amd64", "6.12.9+deb13-amd64", "6.12.116+deb13-amd64"];
        list.sort_by(|a, b| compare_versions(a, b));
        assert_eq!(list, ["6.12.9+deb13-amd64", "6.12.116+deb13-amd64", "6.12.117+deb13-amd64"]);
    }

    #[test]
    fn cmdline_starts_a_subvolume() {
        let line = "root=UUID=0bd0 rw rootflags=compress=zstd:1,noatime quiet splash";
        assert_eq!(
            cmdline_with_subvol(line, "/@"),
            "root=UUID=0bd0 rw rootflags=subvol=/@,compress=zstd:1,noatime quiet splash"
        );
        assert_eq!(
            cmdline_with_subvol("root=UUID=0bd0 rootflags=subvolid=256,noatime", "/@snapshots/9/snapshot"),
            "root=UUID=0bd0 rootflags=subvol=/@snapshots/9/snapshot,noatime"
        );
        assert_eq!(cmdline_with_subvol("root=UUID=0bd0 rw", "/@"), "root=UUID=0bd0 rw rootflags=subvol=/@");
    }

    #[test]
    fn previous_sorts_after() {
        let text = previous_os_release("PRETTY_NAME=\"Orbit Linux 0.1\"\nID=orbit\nIMAGE_ID=x\n", "orbit");
        assert!(text.contains("PRETTY_NAME=\"Orbit Linux 0.1, before going back to a snapshot\""));
        assert!(text.contains("IMAGE_ID=orbit-previous") && !text.contains("IMAGE_ID=x"));
        assert!(text.contains("ID=orbit\n"));
    }
}
