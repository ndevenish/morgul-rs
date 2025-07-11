use bytemuck::{Pod, Zeroable};
use clap::Parser;
use pnet::datalink;
use std::iter;
use std::net::{Ipv4Addr, UdpSocket};
use std::thread;

const MAX_LISTENERS: u16 = 36;

#[repr(C)]
#[derive(Debug, Copy, Clone, Zeroable, Pod)]
struct SlsDetectorHeader {
    /// Frame number to which the current packet belongs to
    frame_number: u64,
    /// Measured exposure time of the frame in tenths of microsecond (100ns)
    exposure_length: u32,
    /// Packet number of the frame to which the current data belongs to.
    packet_number: u32,
    /// detSpec1: Bunch identification number received by the detector at the moment of frame acquisition.
    bunch_id: u64,
    /// Time measured at the start of frame exposure since the start of the current measurement. It is expressed in tenths of microsecond.
    timestamp: u64,
    /// module ID picked up from det_id_[detector type].txt on the detector cpu
    module_id: u16,
    /// row position of the module in the detector system. It is calculated by the order of the module in hostname command, as well as the detsize command. The modules are stacked row by row until they reach the y-axis limit set by detsize (if specified). Then, stacking continues in the next column and so on.
    row: u16,
    /// column position of the module in the detector system. It is calculated by the order of the module in hostname command, as well as the detsize command. The modules are stacked row by row until they reach the y-axis limit set by detsize (if specified). Then, stacking continues in the next column and so on.
    column: u16,
    /// Unused for Jungfrau
    _det_spec_2: u16,
    /// DAQ Info field: See https://slsdetectorgroup.github.io/devdoc/udpdetspec.html#id10
    daq_info: u32,
    /// Unused for Jungfrau
    _det_spec_4: u16,

    /// detector type from enum of detectorType in the package.
    det_type: u8,

    /// Current version of the detector header
    version: u8,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about=None)]
struct Args {
    #[arg(long, short, default_value = "30000")]
    udp_port: u16,
    // #[arg(default_value = "36")]
    // listeners: u16,
}

fn get_interface_addreses_with_prefix(prefix: u8) -> Vec<Ipv4Addr> {
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

fn listen_port(address: &Ipv4Addr, port: u16) -> ! {
    let bind_addr = format!("{address}:{port}");
    let socket = UdpSocket::bind(bind_addr).unwrap();
    let mut buffer = [0u8; 9000];

    println!("{port}: Listening to {address}");
    loop {
        let packet_size = socket.recv(&mut buffer).unwrap();
        let header: &SlsDetectorHeader =
            bytemuck::from_bytes(&buffer[..size_of::<SlsDetectorHeader>()]);
        println!("{port}: Received packet size={packet_size} header={header:?}");
    }
}

fn main() {
    let args = Args::parse();
    println!("Args: {args:?}");

    let interfaces = get_interface_addreses_with_prefix(192);

    let mut threads = Vec::new();
    // Every IP address can cope with 9 streams of data
    for (port, address) in (args.udp_port..(args.udp_port + MAX_LISTENERS)).zip(
        interfaces
            .iter()
            .flat_map(|x| iter::repeat(x.clone()).take(9)),
    ) {
        threads.push(thread::spawn(move || listen_port(&address, port)));
    }

    for thread in threads {
        thread.join().unwrap();
    }
    // thread::spawn(f)
    // let ip = vec![
    //     "192.168.201.101",
    //     "192.168.202.101",
    //     "192.168.203.101",
    //     "192.168.204.101",
    // ];
}
