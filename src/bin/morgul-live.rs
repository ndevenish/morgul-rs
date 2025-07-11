use bytemuck::{Pod, Zeroable};
use clap::Parser;
use pnet::datalink;
use socket2::{Domain, Socket, Type};
use std::fs::soft_link;
use std::io::ErrorKind;
use std::iter;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

use std::thread;
use std::time::Duration;

const MAX_LISTENERS: u16 = 36;
const MODULE_SIZE_X: usize = 1024;
const MODULE_SIZE_Y: usize = 256;
const NUM_PIXELS: usize = MODULE_SIZE_X * MODULE_SIZE_Y;
const BIT_DEPTH: usize = 2;
const THREAD_IMAGE_BUFFER_LENGTH: usize = 10;

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

struct ReceiveImage {
    frame_number: u64,
    header: SlsDetectorHeader,
    received_packets: usize,
    data: Box<[u8]>,
}

impl std::fmt::Debug for ReceiveImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceiveImage")
            .field("frame_number", &self.frame_number)
            .field("header", &self.header)
            .field("received_packets", &self.received_packets)
            .finish()
    }
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

fn allocate_image_buffer() -> Box<[u8]> {
    let mut empty_image = Vec::with_capacity(NUM_PIXELS * BIT_DEPTH);
    empty_image.resize(MODULE_SIZE_X * MODULE_SIZE_Y * BIT_DEPTH, 0u8);
    empty_image.into_boxed_slice()
}

fn listen_port(address: &Ipv4Addr, port: u16) -> ! {
    let bind_addr: SocketAddr = format!("{address}:{port}").parse().unwrap();
    // let socket = UdpSocket::bind(bind_addr).unwrap();
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();
    socket.set_recv_buffer_size(512 * 1024 * 1025).unwrap();
    socket.bind(&bind_addr.into()).unwrap();
    let socket: UdpSocket = socket.into();

    println!("{port}: Listening to {address}");

    // The UDP receive buffer
    let mut buffer = [0u8; size_of::<SlsDetectorHeader>() + 8192];

    // Build the image data buffers we will use
    let mut spare_images: Vec<_> = std::iter::repeat(())
        .take(THREAD_IMAGE_BUFFER_LENGTH)
        .map(|()| allocate_image_buffer())
        .collect();

    let mut last_image = None;
    let mut images_seen = 0usize;
    let mut packets_dropped = 0usize;
    let mut complete_images = 0usize;
    loop {
        let packet_size = match socket.recv(&mut buffer) {
            Ok(size) => size,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                println!(
                    "Got end of acquisition, seen {images_seen} images, {complete_images} complete, {packets_dropped} packets dropped."
                );
                images_seen = 0;
                packets_dropped = 0;
                complete_images = 0;
                socket.set_read_timeout(None).unwrap();
                continue;
            }
            Err(e) => {
                panic!("Error: {e}");
            }
        };
        // Set 500ms timeout so that waits partway through an image fail
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();

        let header: &SlsDetectorHeader =
            bytemuck::from_bytes(&buffer[..size_of::<SlsDetectorHeader>()]);

        assert!(header.packet_number < 64);
        assert!(packet_size - size_of::<SlsDetectorHeader>() == 8192);

        // If no previous image, then we have a new one
        if last_image.is_none() {
            if images_seen == 0 {
                println!(
                    "New Acquisition started with frame number {}",
                    header.frame_number
                );
            }
            images_seen += 1;
        }
        // Get the current WIP image or make a new one
        let mut current_image = last_image.take().unwrap_or_else(|| ReceiveImage {
            frame_number: header.frame_number,
            header: header.clone(),
            received_packets: 0,
            data: spare_images.pop().unwrap(),
        });

        // We have a new image but didn't complete the previous frame
        if header.frame_number != current_image.frame_number {
            // Warn if we received packets for an old image
            if header.frame_number < current_image.frame_number {
                println!(
                    "{port}: Warning: Received Out-Of-Order frame packets for image {} after closing.",
                    header.frame_number
                );
                continue;
            }
            // Warn if we didn't receive the entire previous frame
            if current_image.received_packets < 64 {
                println!(
                    "{port}: Lost packets: Image {} missed {} packets",
                    current_image.frame_number,
                    64 - current_image.received_packets
                );
                packets_dropped += 64 - current_image.received_packets;
                // Return the data back to the pool to simulate sending it
                spare_images.push(current_image.data);
            }
            // Even though we didn't complete the previous image, this is a new one
            images_seen += 1;
            // Make a new image
            current_image = ReceiveImage {
                frame_number: header.frame_number,
                header: header.clone(),
                received_packets: 0,
                data: spare_images.pop().unwrap(),
            }
        }
        assert!(header.frame_number == current_image.frame_number);

        // Add a packet to this image
        current_image.received_packets += 1;
        // Copy the new data into the image data at the right place
        current_image.data[(header.packet_number as usize * 8192usize)
            ..((header.packet_number as usize + 1) * 8192usize)]
            .copy_from_slice(&buffer[size_of::<SlsDetectorHeader>()..]);

        // If we've received an entire image, then process it
        if current_image.received_packets == 64 {
            // println!(
            //     "{port}: Received entire image {}",
            //     current_image.frame_number
            // );
            spare_images.push(current_image.data);
            last_image = None;
            complete_images += 1;
            // socket.set_read_timeout(None).unwrap();
        } else {
            last_image = Some(current_image);
        }
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
