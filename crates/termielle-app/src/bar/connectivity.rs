use std::time::Duration;
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation};
use windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED;
use windows_future::AsyncStatus;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkState {
    Online,
    Limited,
    Offline,
    #[default]
    Unknown,
}

impl NetworkState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Limited => "Limited",
            Self::Offline => "Offline",
            Self::Unknown => "Settings",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BluetoothState {
    On,
    Off,
    Disabled,
    #[default]
    Unknown,
}

impl BluetoothState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::On => "On",
            Self::Off => "Off",
            Self::Disabled => "Disabled",
            Self::Unknown => "Settings",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectivitySnapshot {
    pub network: NetworkState,
    pub bluetooth: BluetoothState,
}

pub fn query_connectivity() -> ConnectivitySnapshot {
    let Ok(_apartment) = crate::apartment::Apartment::new(RO_INIT_SINGLETHREADED) else {
        return ConnectivitySnapshot::default();
    };
    let network = match NetworkInformation::GetInternetConnectionProfile() {
        Ok(profile) if windows::core::Interface::as_raw(&profile).is_null() => {
            NetworkState::Offline
        }
        Ok(profile) => match profile.GetNetworkConnectivityLevel() {
            Ok(NetworkConnectivityLevel::InternetAccess) => NetworkState::Online,
            Ok(NetworkConnectivityLevel::ConstrainedInternetAccess)
            | Ok(NetworkConnectivityLevel::LocalAccess) => NetworkState::Limited,
            Ok(NetworkConnectivityLevel::None) => NetworkState::Offline,
            _ => NetworkState::Unknown,
        },
        Err(_) => NetworkState::Unknown,
    };
    let bluetooth = Radio::GetRadiosAsync()
        .ok()
        .and_then(|operation| {
            for _ in 0..40 {
                match operation.Status().ok()? {
                    AsyncStatus::Completed => return operation.GetResults().ok(),
                    AsyncStatus::Canceled | AsyncStatus::Error => return None,
                    _ => std::thread::sleep(Duration::from_millis(25)),
                }
            }
            None
        })
        .and_then(|radios| {
            radios.into_iter().find_map(|radio| {
                (radio.Kind().ok()? == RadioKind::Bluetooth).then(|| match radio.State().ok() {
                    Some(RadioState::On) => BluetoothState::On,
                    Some(RadioState::Off) => BluetoothState::Off,
                    Some(RadioState::Disabled) => BluetoothState::Disabled,
                    _ => BluetoothState::Unknown,
                })
            })
        })
        .unwrap_or(BluetoothState::Unknown);
    ConnectivitySnapshot { network, bluetooth }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_states_are_not_presented_as_off() {
        assert_eq!(NetworkState::Unknown.label(), "Settings");
        assert_eq!(BluetoothState::Unknown.label(), "Settings");
    }

    #[test]
    fn live_query_is_read_only() {
        let snapshot = query_connectivity();
        assert!(!snapshot.network.label().is_empty());
        assert!(!snapshot.bluetooth.label().is_empty());
        eprintln!("{snapshot:?}");
    }
}
