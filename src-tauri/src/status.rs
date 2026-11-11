//! State behind the status icons: network, volume, battery.

use serde::Serialize;
use std::cell::Cell;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, IP_ADAPTER_ADDRESSES_LH};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::AF_UNSPEC;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Status {
    pub network: Network,
    /// 0-4 bars on Wi-Fi.
    // shortcut: always None; needs the WLAN API (WlanQueryInterface), add when the Wi-Fi icon shows bars.
    pub wifi_signal: Option<u8>,
    pub volume: u8,
    pub muted: bool,
    pub battery: Option<Battery>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    None,
    Ethernet,
    Wifi,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// Cheap enough to call every 2 s from a background thread.
pub fn read() -> Status {
    let (volume, muted) = volume().unwrap_or((0, true));
    Status { network: network(), wifi_signal: None, volume, muted, battery: battery() }
}

/// Battery from the power-status flags; None without a battery (flag 128) or when unknown (255).
fn battery_from(flag: u8, percent: u8, ac_online: u8) -> Option<Battery> {
    if flag & 128 != 0 || percent > 100 {
        return None;
    }
    Some(Battery { percent, charging: ac_online == 1 })
}

fn battery() -> Option<Battery> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s) }.ok()?;
    battery_from(s.BatteryFlag, s.BatteryLifePercent, s.ACLineStatus)
}

/// Wi-Fi wins over Ethernet when both are up.
fn network_from(up_types: impl Iterator<Item = u32>) -> Network {
    let mut net = Network::None;
    for t in up_types {
        match t {
            71 => return Network::Wifi,
            6 => net = Network::Ethernet,
            _ => {}
        }
    }
    net
}

// shortcut: "connected" = adapter up with a gateway, no internet check (INetworkListManager); fine for the icon.
fn network() -> Network {
    let mut buf = vec![0u64; 2048]; // 16 KB, 8-byte aligned
    let mut len = (buf.len() * 8) as u32;
    let first = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
    let err = unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, GAA_FLAG_INCLUDE_GATEWAYS, None, Some(first), &mut len) };
    if err == 111 {
        // ERROR_BUFFER_OVERFLOW: retry once with the size Windows asked for.
        buf = vec![0u64; len as usize / 8 + 1];
        let first = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        if unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, GAA_FLAG_INCLUDE_GATEWAYS, None, Some(first), &mut len) } != 0 {
            return Network::None;
        }
    } else if err != 0 {
        return Network::None;
    }
    let mut types = Vec::new();
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while let Some(a) = unsafe { p.as_ref() } {
        if a.OperStatus == IfOperStatusUp && !a.FirstGatewayAddress.is_null() {
            types.push(a.IfType);
        }
        p = a.Next;
    }
    network_from(types.into_iter())
}

thread_local!(static COM: Cell<bool> = const { Cell::new(false) });

/// Default output device's volume (0-100) and mute; None without a device.
fn volume() -> Option<(u8, bool)> {
    COM.with(|c| {
        if !c.replace(true) {
            // Already-initialised (S_FALSE / other mode) is fine for our calls.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        }
    });
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let vol: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
        let level = vol.GetMasterVolumeLevelScalar().ok()?;
        let muted = vol.GetMute().ok()?.as_bool();
        Some(((level * 100.0).round() as u8, muted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_flags_and_adapters() {
        assert_eq!(battery_from(128, 255, 1), None);
        assert_eq!(battery_from(8, 80, 1), Some(Battery { percent: 80, charging: true }));
        assert_eq!(network_from([6, 71].into_iter()), Network::Wifi);
        assert_eq!(network_from([24, 6].into_iter()), Network::Ethernet);
        assert_eq!(network_from([24].into_iter()), Network::None);
    }

    #[test]
    #[ignore]
    fn print_status() {
        println!("{:?}", read());
    }
}
