//! New Wi-Fi credentials travel only in NetworkManager's D-Bus payload.
//! API: https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html

use std::collections::HashMap;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, Value};

const NM: &str = "org.freedesktop.NetworkManager";
type Settings<'a> = HashMap<&'static str, HashMap<&'static str, Value<'a>>>;

fn connection_settings<'a>(ssid: &'a str, password: Option<&'a str>, wep: bool) -> Result<Settings<'a>, String> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err("A network name must contain 1 to 32 bytes".into());
    }
    if password.is_some_and(|secret| secret.is_empty() || secret.len() > 128 || secret.chars().any(char::is_control)) {
        return Err("The Wi-Fi password is empty, too long or contains control characters".into());
    }
    let mut settings = HashMap::from([
        ("connection", HashMap::from([
            ("id", Value::from(ssid)),
            ("type", Value::from("802-11-wireless")),
            ("autoconnect", Value::from(true)),
        ])),
        ("802-11-wireless", HashMap::from([("ssid", Value::from(ssid.as_bytes().to_vec()))])),
    ]);
    if let Some(secret) = password {
        let mut security = HashMap::from([(if wep { "wep-key0" } else { "psk" }, Value::from(secret))]);
        if wep {
            let raw = matches!(secret.len(), 5 | 13) || (matches!(secret.len(), 10 | 26) && secret.bytes().all(|c| c.is_ascii_hexdigit()));
            security.insert("wep-key-type", Value::from(if raw { 1u32 } else { 2u32 }));
        }
        // NetworkManager completes key-mgmt from the chosen AP, including SAE.
        settings.insert("802-11-wireless-security", security);
    }
    Ok(settings)
}

fn proxy<'a>(bus: &'a Connection, path: &'a str, interface: &'a str) -> Result<Proxy<'a>, String> {
    Proxy::new(bus, NM, path, interface).map_err(|_| "Cannot contact NetworkManager".into())
}

/// Runs on the radio worker. Never format a D-Bus error that might include secrets.
pub(super) fn connect(ssid: &str, password: Option<&str>) -> Result<(), String> {
    connection_settings(ssid, password, false)?;
    let bus = zbus::blocking::connection::Builder::system()
        .map_err(|_| "Cannot open the system bus")?
        .method_timeout(Duration::from_secs(5))
        .build().map_err(|_| "Cannot open the system bus")?;
    let manager = proxy(&bus, "/org/freedesktop/NetworkManager", NM)?;
    let devices: Vec<OwnedObjectPath> = manager.call("GetDevices", &())
        .map_err(|_| "Cannot read network devices")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    for device in devices {
        let generic = proxy(&bus, device.as_str(), "org.freedesktop.NetworkManager.Device")?;
        if generic.get_property::<u32>("DeviceType").ok() != Some(2) { continue; }
        let wireless = proxy(&bus, device.as_str(), "org.freedesktop.NetworkManager.Device.Wireless")?;
        let aps: Vec<OwnedObjectPath> = wireless.call("GetAccessPoints", &()).map_err(|_| "Cannot read nearby networks")?;
        for ap in aps {
            if Instant::now() >= deadline { return Err("Finding the Wi-Fi network timed out".into()); }
            let access = proxy(&bus, ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint")?;
            if access.get_property::<Vec<u8>>("Ssid").ok().as_deref() != Some(ssid.as_bytes()) { continue; }
            let flags: u32 = access.get_property("Flags").map_err(|_| "Cannot read network security")?;
            let wpa: u32 = access.get_property("WpaFlags").map_err(|_| "Cannot read network security")?;
            let rsn: u32 = access.get_property("RsnFlags").map_err(|_| "Cannot read network security")?;
            let security = wpa | rsn;
            if security & (0x200 | 0x2000) != 0 && security & (0x100 | 0x400) == 0 {
                return Err("This network needs enterprise authentication; configure it with NetworkManager first".into());
            }
            let settings = connection_settings(ssid, password, security == 0 && flags & 1 != 0)?;
            let (profile, active): (OwnedObjectPath, OwnedObjectPath) = manager
                .call("AddAndActivateConnection", &(settings, &device, &ap))
                .map_err(|_| "NetworkManager refused the Wi-Fi connection; check the password and your permissions")?;
            let outcome = wait_for_connection(&bus, &active);
            if outcome.is_err() {
                // Delete only the profile this request created, allowing a fresh
                // password on retry rather than leaving a broken saved network.
                if let Ok(profile) = proxy(&bus, profile.as_str(), "org.freedesktop.NetworkManager.Settings.Connection") {
                    let _ = profile.call::<_, _, ()>("Delete", &());
                }
            }
            return outcome;
        }
    }
    Err("The Wi-Fi network is no longer in range; scan again".into())
}

fn wait_for_connection(bus: &Connection, path: &OwnedObjectPath) -> Result<(), String> {
    let active = proxy(bus, path.as_str(), "org.freedesktop.NetworkManager.Connection.Active")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match active.get_property::<u32>("State") {
            Ok(2) => return Ok(()),
            Ok(3 | 4) | Err(_) => return Err("Wi-Fi connection failed; check the password and signal".into()),
            _ if Instant::now() >= deadline => return Err("Wi-Fi connection timed out".into()),
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    #[test]
    fn builds_a_connection_without_interpreting_ssid_or_secret_as_arguments() {
        let settings = connection_settings("Cafe: \"$HOME\"", Some("a:b\\c secret"), false).unwrap();
        let wireless = &settings["802-11-wireless"];
        let ssid: Vec<u8> = wireless["ssid"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(ssid, b"Cafe: \"$HOME\"");
        assert_eq!(<&str>::try_from(&settings["802-11-wireless-security"]["psk"]).unwrap(), "a:b\\c secret");
        assert_eq!(settings["connection"]["autoconnect"], Value::from(true));
    }

    #[test]
    fn open_network_has_no_secret_and_wep_uses_its_own_property() {
        let open = connection_settings("Cafe", None, false).unwrap();
        assert!(!open.contains_key("802-11-wireless-security"));
        let wep = connection_settings("Legacy", Some("abcde"), true).unwrap();
        let security = &wep["802-11-wireless-security"];
        assert!(!security.contains_key("psk"));
        assert_eq!(<&str>::try_from(&security["wep-key0"]).unwrap(), "abcde");
    }

    #[test]
    fn invalid_ssids_and_control_characters_never_reach_networkmanager() {
        for ssid in ["", &"x".repeat(33)] {
            assert!(connection_settings(ssid, None, false).is_err());
        }
        for secret in ["", "password\nnext", "password\0next"] {
            let error = connection_settings("Home", Some(secret), false).unwrap_err();
            assert!(!error.contains(secret) || secret.is_empty());
        }
    }
}
