use clap::Parser;
use morgul::{SlsDetectorHeader, get_interface_addreses_with_prefix};
use nix::errno::Errno;
use nix::sys::socket::{
    ControlMessageOwned, MsgFlags, SockaddrStorage, recvmsg, setsockopt, sockopt,
};

use socket2::{Domain, Socket, Type};
use std::io::IoSliceMut;
use std::iter;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Barrier};
use thread_priority::set_current_thread_priority;

use std::thread;
use std::time::Duration;

const MAX_LISTENERS: u16 = 36;
const MODULE_SIZE_X: usize = 1024;
const MODULE_SIZE_Y: usize = 256;
const NUM_PIXELS: usize = MODULE_SIZE_X * MODULE_SIZE_Y;
const BIT_DEPTH: usize = 2;
const THREAD_IMAGE_BUFFER_LENGTH: usize = 10;

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

fn allocate_image_buffer() -> Box<[u8]> {
    let mut empty_image = Vec::with_capacity(NUM_PIXELS * BIT_DEPTH);
    empty_image.resize(MODULE_SIZE_X * MODULE_SIZE_Y * BIT_DEPTH, 0u8);
    empty_image.into_boxed_slice()
}

fn listen_port(address: &Ipv4Addr, port: u16, barrier: Arc<Barrier>) -> ! {
    if set_current_thread_priority(thread_priority::ThreadPriority::Max).is_err() {
        println!("{port}: Warning: Could not set thread priority. Are you running as root?");
    };
    let bind_addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    // let socket = UdpSocket::bind(bind_addr).unwrap();
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();
    socket.set_recv_buffer_size(1024 * 1024 * 1024).unwrap();
    socket.bind(&bind_addr.into()).unwrap();
    setsockopt(&socket, sockopt::RxqOvfl, &1).unwrap();
    let socket: UdpSocket = socket.into();
    println!("{port}: Listening to {address}");

    // The UDP receive buffer
    let mut buffer = [0u8; size_of::<SlsDetectorHeader>() + 8192];

    // let mut buf = vec![0u8; 1500];
    // let mut cmsgspace = nix::cmsg_space!(libc::c_uint);

    // let msg = recvmsg::<SockaddrStorage>(
    //     fd,
    //     &[nix::sys::uio::IoVec::from_mut_slice(&mut buf)],
    //     Some(&mut cmsgspace),
    //     MsgFlags::empty(),
    // )
    // .expect("recvmsg failed");

    // for cmsg in msg.cmsgs() {
    //     if let ControlMessageOwned::RxqOvfl(count) = cmsg {
    //         println!("Packet queue overflowed! {} packets dropped.", count);
    //     }
    // }
    let fd = socket.as_raw_fd();
    let mut iov = [IoSliceMut::new(&mut buffer)];
    let mut cmsgspace = nix::cmsg_space!(libc::c_uint);

    // Build the image data buffers we will use
    let mut spare_images: Vec<_> = std::iter::repeat_n((), THREAD_IMAGE_BUFFER_LENGTH)
        .map(|()| allocate_image_buffer())
        .collect();

    let mut last_image = None;
    let mut images_seen = 0usize;
    let mut packets_dropped = 0usize;
    let mut complete_images = 0usize;
    let mut out_of_order = 0usize;

    loop {
        // barrier.wait();
        let msg = match recvmsg::<SockaddrStorage>(
            fd,
            &mut iov,
            Some(&mut cmsgspace),
            MsgFlags::empty(),
        ) {
            Ok(msg) => msg,
            Err(Errno::EAGAIN) => {
                println!(
                    "{port}: End of acquisition, seen {images_seen} images, {complete_images} complete, {packets_dropped} packets dropped, {out_of_order} out-of-order."
                );
                images_seen = 0;
                packets_dropped = 0;
                complete_images = 0;
                out_of_order = 0;
                socket.set_read_timeout(None).unwrap();
                let b = barrier.wait();
                if b.is_leader() {
                    println!("All threads finished acquisition.");
                }
                continue;
            }
            Err(e) => {
                panic!("Error: {e}");
            }
        };

        for cmsg in msg.cmsgs().unwrap() {
            if let ControlMessageOwned::RxqOvfl(count) = cmsg {
                println!("{port}: Packet queue overflowed! {count} packets dropped.",);
            } else {
                println!("{port}: Got ControlMessage: {cmsg:?}");
            }
        }

        // Unwrap the buffer
        let buffer = msg.iovs().next().unwrap();
        let packet_size = msg.bytes;

        // let packet_size = match socket.recv(&mut buffer) {
        //     Ok(size) => size,
        //     Err(e) if e.kind() == ErrorKind::WouldBlock => {
        //         println!(
        //             "{port}: End of acquisition, seen {images_seen} images, {complete_images} complete, {packets_dropped} packets dropped."
        //         );
        //         images_seen = 0;
        //         packets_dropped = 0;
        //         complete_images = 0;
        //         socket.set_read_timeout(None).unwrap();
        //         continue;
        //     }
        //     Err(e) => {
        //         panic!("Error: {e}");
        //     }
        // };
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
            header: *header,
            received_packets: 0,
            data: spare_images.pop().expect("Ran out of spare packet buffers"),
        });

        // We have a new image but didn't complete the previous frame
        if header.frame_number != current_image.frame_number {
            // Warn if we received packets for an old image
            if header.frame_number < current_image.frame_number {
                // println!(
                //     "{port}: Warning: Received Out-Of-Order frame packets for image {} (current={}) after closing.",
                //     header.frame_number, current_image.frame_number,
                // );

                out_of_order += 1;
                last_image = Some(current_image);
                continue;
            }
            // Warn if we didn't receive the entire previous frame
            if current_image.received_packets < 64 {
                // println!(
                //     "{port}: Lost packets: Image {} missed {} packets",
                //     current_image.frame_number,
                //     64 - current_image.received_packets
                // );
                packets_dropped += 64 - current_image.received_packets;
                // Return the data back to the pool to simulate sending it
                spare_images.push(current_image.data);
            }
            // Even though we didn't complete the previous image, this is a new one
            images_seen += 1;
            // Make a new image
            current_image = ReceiveImage {
                frame_number: header.frame_number,
                header: *header,
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
    if interfaces.is_empty() {
        println!("Error: Could not find any 192. interfaces. Have you set up the network?");
        std::process::exit(1);
    }
    // Get a list of cores so that we can set affinity to them
    let mut core_ids = core_affinity::get_core_ids().unwrap().into_iter().rev();
    println!("{core_ids:?}");
    println!("Start threads");

    let barrier = Arc::new(Barrier::new(std::cmp::min(
        interfaces.len() * 9,
        MAX_LISTENERS as usize,
    )));

    let mut threads = Vec::new();
    // Every IP address can cope with 9 streams of data
    for (port, address) in (args.udp_port..(args.udp_port + MAX_LISTENERS))
        .zip(interfaces.iter().flat_map(|x| iter::repeat_n(*x, 9)))
    {
        let core = core_ids.next().unwrap();
        let barr = barrier.clone();
        threads.push(thread::spawn(move || {
            if !core_affinity::set_for_current(core) {
                println!("{port}: Failed to set affinity to core {}", core.id);
            } else {
                println!("{port}: Setting affinity to CPU {}", core.id);
            }
            listen_port(&address, port, barr);
        }));
    }

    loop {
        thread::sleep(Duration::from_secs(20));
    }
    // #[allow(clippy::never_loop)]
    // for thread in threads {
    //     thread.join().unwrap();
    // }
    // thread::spawn(f)
    // let ip = vec![
    //     "192.168.201.101",
    //     "192.168.202.101",
    //     "192.168.203.101",
    //     "192.168.204.101",
    // ];
}
