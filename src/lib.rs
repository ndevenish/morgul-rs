use std::net::Ipv4Addr;

use bytemuck::{Pod, Zeroable};
use pnet::datalink;

#[repr(C)]
#[derive(Debug, Copy, Clone, Zeroable, Pod)]
pub struct DelugeTrigger {
    pub frames: u128,
    pub exptime: f32,
    pub uuid: [u8; 12],
}
impl Default for DelugeTrigger {
    fn default() -> Self {
        DelugeTrigger {
            frames: 0,
            exptime: 0.0,
            uuid: rand::random(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Zeroable, Pod)]
pub struct SlsDetectorHeader {
    /// Frame number to which the current packet belongs to
    pub frame_number: u64,
    /// Measured exposure time of the frame in tenths of microsecond (100ns)
    pub exposure_length: u32,
    /// Packet number of the frame to which the current data belongs to.
    pub packet_number: u32,
    /// detSpec1: Bunch identification number received by the detector at the moment of frame acquisition.
    pub bunch_id: u64,
    /// Time measured at the start of frame exposure since the start of the current measurement. It is expressed in tenths of microsecond.
    pub timestamp: u64,
    /// module ID picked up from det_id_[detector type].txt on the detector cpu
    pub module_id: u16,
    /// row position of the module in the detector system. It is calculated by the order of the module in hostname command, as well as the detsize command. The modules are stacked row by row until they reach the y-axis limit set by detsize (if specified). Then, stacking continues in the next column and so on.
    pub row: u16,
    /// column position of the module in the detector system. It is calculated by the order of the module in hostname command, as well as the detsize command. The modules are stacked row by row until they reach the y-axis limit set by detsize (if specified). Then, stacking continues in the next column and so on.
    pub column: u16,
    /// Unused for Jungfrau
    _det_spec_2: u16,
    /// DAQ Info field: See https://slsdetectorgroup.github.io/devdoc/udpdetspec.html#id10
    pub daq_info: u32,
    /// Unused for Jungfrau
    _det_spec_4: u16,

    /// detector type from enum of detectorType in the package.
    pub det_type: u8,

    /// Current version of the detector header
    pub version: u8,
}

pub enum SlsDetectorType {
    Generic = 0,
    Eiger = 1,
    Gotthard = 2,
    Jungfrau = 3,
    ChipTestBoard = 4,
    Moench = 5,
    Mythen3 = 6,
    Gotthard2 = 7,
}

impl TryFrom<u8> for SlsDetectorType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(SlsDetectorType::Generic),
            1 => Ok(SlsDetectorType::Eiger),
            2 => Ok(SlsDetectorType::Gotthard),
            3 => Ok(SlsDetectorType::Jungfrau),
            4 => Ok(SlsDetectorType::ChipTestBoard),
            5 => Ok(SlsDetectorType::Moench),
            6 => Ok(SlsDetectorType::Mythen3),
            7 => Ok(SlsDetectorType::Gotthard2),
            _ => Err(()),
        }
    }
}

pub fn get_interface_addreses_with_prefix(prefix: u8) -> Vec<Ipv4Addr> {
    let mut addresses: Vec<_> = datalink::interfaces()
        .iter()
        .flat_map(|x| &x.ips)
        .flat_map(|x| match x {
            pnet::ipnetwork::IpNetwork::V4(ip) => Some(ip),
            _ => None,
        })
        .map(|x| x.ip())
        .filter(|x| x.octets()[0] == prefix)
        .collect();
    addresses.sort();
    addresses
}
