//! Linux network counters from /sys/class/net.

use crate::counters::AdapterInfo;
use std::fs;
use std::path::Path;

/// `ARPHRD_LOOPBACK` from linux/if_arp.h, as reported in `type`.
const ARPHRD_LOOPBACK: &str = "772";

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Every interface under `<sys_root>/class/net`, sorted by name. Interfaces
/// whose byte counters can't be read (e.g. unplugged mid-read) are skipped.
pub fn read_adapters_from(sys_root: &Path) -> Result<Vec<AdapterInfo>, String> {
    let net = sys_root.join("class/net");
    let entries = fs::read_dir(&net).map_err(|e| format!("{}: {e}", net.display()))?;
    let mut list = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path();
        let counter = |file: &str| read_trimmed(&dir.join("statistics").join(file))?.parse::<u64>().ok();
        let (Some(rx), Some(tx)) = (counter("rx_bytes"), counter("tx_bytes")) else { continue };
        let operstate = read_trimmed(&dir.join("operstate")).unwrap_or_default();
        // Many Wi-Fi and tunnel drivers report "unknown"; trust the carrier then.
        let is_up = operstate == "up"
            || (operstate == "unknown" && read_trimmed(&dir.join("carrier")).as_deref() == Some("1"));
        list.push(AdapterInfo {
            guid: name.clone(),
            alias: name,
            description: String::new(),
            is_up,
            // Physical NICs have a backing device; bridges, veth, VPNs and lo don't.
            is_hardware: dir.join("device").exists(),
            is_loopback: read_trimmed(&dir.join("type")).as_deref() == Some(ARPHRD_LOOPBACK),
            is_filter: false,
            in_octets: rx,
            out_octets: tx,
        });
    }
    list.sort_by(|a, b| a.guid.cmp(&b.guid));
    Ok(list)
}

/// Interfaces of this machine.
pub fn read_adapters() -> Result<Vec<AdapterInfo>, String> {
    read_adapters_from(Path::new("/sys"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ferrimeter-sysfs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("class/net")).unwrap();
        d
    }

    #[allow(clippy::too_many_arguments)]
    fn iface(root: &Path, name: &str, operstate: &str, carrier: Option<&str>, type_: &str, device: bool, rx: Option<&str>, tx: Option<&str>) {
        let dir = root.join("class/net").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("operstate"), format!("{operstate}\n")).unwrap();
        if let Some(c) = carrier {
            fs::write(dir.join("carrier"), format!("{c}\n")).unwrap();
        }
        fs::write(dir.join("type"), format!("{type_}\n")).unwrap();
        if device {
            fs::create_dir_all(dir.join("device")).unwrap();
        }
        if rx.is_some() || tx.is_some() {
            fs::create_dir_all(dir.join("statistics")).unwrap();
        }
        if let Some(v) = rx {
            fs::write(dir.join("statistics/rx_bytes"), v).unwrap();
        }
        if let Some(v) = tx {
            fs::write(dir.join("statistics/tx_bytes"), v).unwrap();
        }
    }

    #[test]
    fn physical_up_qualifies() {
        let root = temp_root("physical");
        iface(&root, "enp3s0", "up", Some("1"), "1", true, Some("100\n"), Some("7\n"));
        let a = &read_adapters_from(&root).unwrap()[0];
        assert_eq!((a.guid.as_str(), a.alias.as_str()), ("enp3s0", "enp3s0"));
        assert!(a.is_up && a.is_hardware && !a.is_loopback && !a.is_filter);
        assert_eq!((a.in_octets, a.out_octets), (100, 7));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unknown_operstate_with_carrier_is_up() {
        let root = temp_root("unknown");
        iface(&root, "wlan0", "unknown", Some("1"), "1", true, Some("1"), Some("1"));
        iface(&root, "wlan1", "unknown", Some("0"), "1", true, Some("1"), Some("1"));
        let list = read_adapters_from(&root).unwrap();
        assert!(list[0].is_up);
        assert!(!list[1].is_up);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn loopback_detected() {
        let root = temp_root("lo");
        iface(&root, "lo", "unknown", Some("1"), "772", false, Some("5"), Some("5"));
        assert!(read_adapters_from(&root).unwrap()[0].is_loopback);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn virtual_interfaces_are_not_hardware() {
        let root = temp_root("virtual");
        iface(&root, "docker0", "up", Some("1"), "1", false, Some("1"), Some("1"));
        iface(&root, "wg0", "unknown", Some("1"), "65534", false, Some("1"), Some("1"));
        assert!(read_adapters_from(&root).unwrap().iter().all(|a| !a.is_hardware));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn interface_without_statistics_skipped() {
        let root = temp_root("nostats");
        iface(&root, "eth9", "up", Some("1"), "1", true, None, None);
        iface(&root, "eth0", "up", Some("1"), "1", true, Some("1"), Some("1"));
        let names: Vec<_> = read_adapters_from(&root).unwrap().into_iter().map(|a| a.guid).collect();
        assert_eq!(names, ["eth0"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parses_max_u64_with_newline() {
        let root = temp_root("maxu64");
        iface(&root, "eth0", "up", Some("1"), "1", true, Some("18446744073709551615\n"), Some("0\n"));
        assert_eq!(read_adapters_from(&root).unwrap()[0].in_octets, u64::MAX);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_root_is_error() {
        assert!(read_adapters_from(Path::new("/nonexistent-ferrimeter-root")).is_err());
    }

    #[test]
    fn sorted_by_name() {
        let root = temp_root("sorted");
        for n in ["wlan0", "eth0", "lo"] {
            iface(&root, n, "up", Some("1"), "1", true, Some("1"), Some("1"));
        }
        let names: Vec<_> = read_adapters_from(&root).unwrap().into_iter().map(|a| a.guid).collect();
        assert_eq!(names, ["eth0", "lo", "wlan0"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reads_real_sys() {
        assert!(read_adapters().unwrap().iter().any(|a| a.guid == "lo" && a.is_loopback));
    }
}
