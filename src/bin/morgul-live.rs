use clap::Parser;
use itertools::multizip;
use morgul::{SlsDetectorHeader, get_interface_addreses_with_prefix};
use nix::errno::Errno;
use nix::sys::socket::{
    ControlMessageOwned, MsgFlags, RecvMsg, SockaddrStorage, recvmsg, setsockopt, sockopt,
};

use socket2::{Domain, Socket, Type};
use std::io::IoSliceMut;
use std::iter;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Barrier, mpsc};
use thread_priority::set_current_thread_priority;

use std::thread;
use std::time::Duration;

const LISTENERS_PER_PORT: usize = 9;
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

/// Elect a leader by selecting the first thread through.
///
/// The intention of how this differs from a standard [Barrier] in that
/// it elects a leader but does not require a fixed number of threads
/// known up-front at creation time. This is important because some of
/// the senders might fail to send or be blocked (e.g. a module dies),
/// and we don't want the entire data pipeline to fail in these cases.
// #[derive(Debug)]
// struct IsFirstThread {
//     is_first: AtomicBool,
// }

// impl IsFirstThread {
//     fn try_claim(&mut self) -> bool {
//         self.is_first
//             .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
//             .is_ok()
//     }
//     fn reset(&self) {
//         self.is_first.store(false, Ordering::SeqCst);
//     }
// }

static ACQUISITION_NUMBER: AtomicUsize = AtomicUsize::new(0usize);

#[derive(Debug, Default)]
struct AcquisitionStats {
    /// How many images have we seen at least one packet for
    images_seen: usize,
    /// How many images received all packet data
    complete_images: usize,
    /// How many packets were we expecting but didn't arrive
    packets_dropped: usize,
    /// How many packets did we get too late to assemble
    out_of_order: usize,
}

/// For reporting ongoing progress/statistics to a central thread
enum AcquisitionLifecycleState {
    /// An acquisition task is starting, along with the acquisition ID
    Starting { acquisition_number: usize },
    ImageReceived {
        image_number: usize,
        dropped_packets: usize,
    },
    /// An acquisition was ended by a thread
    Ended(AcquisitionStats),
}

/// Start a UDP socket, with custom options
///
/// At the moment this is just
///   - Turn on RX
fn start_socket(address: SocketAddr, buffer_size: usize) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
    socket.set_recv_buffer_size(buffer_size)?;
    socket.bind(&address.into())?;
    setsockopt(&socket, sockopt::RxqOvfl, &1)?;
    Ok(socket.into())
}

trait RecvMessageWrapper {
    fn get_dropped_packets(&self) -> nix::Result<usize>;
}
impl<'a, 's, S> RecvMessageWrapper for RecvMsg<'a, 's, S> {
    fn get_dropped_packets(&self) -> nix::Result<usize> {
        for cmsg in self.cmsgs()? {
            if let ControlMessageOwned::RxqOvfl(count) = cmsg {
                return Ok(count as usize);
            }
        }
        Ok(0)
    }
}

fn listen_port(
    address: &Ipv4Addr,
    port: u16,
    barrier: Arc<Barrier>,
    state_report: Sender<(u16, AcquisitionLifecycleState)>,
) -> ! {
    let bind_addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    let socket = start_socket(bind_addr, 512 * 1024 * 1024).unwrap();
    println!("{port}: Listening to {address}");

    // The UDP receive buffer
    let mut buffer = [0u8; size_of::<SlsDetectorHeader>() + 8192];

    let fd = socket.as_raw_fd();
    let mut iov = [IoSliceMut::new(&mut buffer)];
    let mut cmsgspace = nix::cmsg_space!(libc::c_uint);

    // Build the image data buffers we will use
    let mut spare_images: Vec<_> = std::iter::repeat_n((), THREAD_IMAGE_BUFFER_LENGTH)
        .map(|()| allocate_image_buffer())
        .collect();

    loop {
        let mut stats = AcquisitionStats::default();
        let mut last_image = None;
        let acquisition_number = ACQUISITION_NUMBER.load(Ordering::Relaxed);

        // Wait forever for the first image in an acquisition
        socket.set_read_timeout(None).unwrap();

        // Many images in one acquisition
        loop {
            let msg = match recvmsg::<SockaddrStorage>(
                fd,
                &mut iov,
                Some(&mut cmsgspace),
                MsgFlags::empty(),
            ) {
                Ok(msg) => msg,
                Err(Errno::EAGAIN) => break,
                Err(e) => {
                    panic!("Error: {e}");
                }
            };

            if let Ok(dropped) = msg.get_dropped_packets()
                && dropped > 0
            {
                stats.packets_dropped += dropped;
                println!("{port}: Packet queue overflowed! {dropped} packets dropped!");
            }
            // Is this the start of a new acquisition?
            if last_image.is_none() {
                // Once we have started an acquisition, we want to expire it when the images stop
                socket
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                state_report
                    .send((
                        port,
                        AcquisitionLifecycleState::Starting { acquisition_number },
                    ))
                    .unwrap();
            }

            // Unwrap the buffer
            let buffer = msg.iovs().next().unwrap();

            let header: &SlsDetectorHeader =
                bytemuck::from_bytes(&buffer[..size_of::<SlsDetectorHeader>()]);

            assert!(header.packet_number < 64);
            assert!(msg.bytes - size_of::<SlsDetectorHeader>() == 8192);

            // If no previous image, then we have a new one
            if last_image.is_none() {
                if stats.images_seen == 0 {
                    println!(
                        "New Acquisition started with frame number {}",
                        header.frame_number
                    );
                }
                stats.images_seen += 1;
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

                    stats.out_of_order += 1;
                    stats.packets_dropped -= 1;
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
                    stats.packets_dropped += 64 - current_image.received_packets;
                    // Return the data back to the pool to simulate sending it
                    spare_images.push(current_image.data);
                }
                // Even though we didn't complete the previous image, this is a new one
                stats.images_seen += 1;
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
                stats.complete_images += 1;
                // socket.set_read_timeout(None).unwrap();
            } else {
                last_image = Some(current_image);
            }
        } // Acquisition loop

        println!(
            "{port}: End of acquisition, seen {is} images, {ci} complete, {pd} packets dropped, {ooo} out-of-order.",
            is = stats.images_seen,
            ci = stats.complete_images,
            pd = stats.packets_dropped,
            ooo = stats.out_of_order
        );
        let b = barrier.wait();
        if b.is_leader() {
            println!("All threads finished acquisition.");
        }
        continue;
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
    let num_listeners = interfaces.len() * LISTENERS_PER_PORT;

    // Get a list of cores so that we can set affinity to them
    let mut core_ids = core_affinity::get_core_ids().unwrap().into_iter().rev();

    let barrier = Arc::new(Barrier::new(num_listeners));
    let (state_tx, state_rx) = mpsc::channel::<(u16, AcquisitionLifecycleState)>();

    let mut threads = Vec::new();

    for (port, address) in multizip((
        args.udp_port..(args.udp_port + num_listeners as u16),
        interfaces
            .iter()
            .flat_map(|x| iter::repeat_n(*x, LISTENERS_PER_PORT)),
    )) {
        let core = core_ids.next().unwrap();
        let barr = barrier.clone();
        let stat = state_tx.clone();
        threads.push(thread::spawn(move || {
            if !core_affinity::set_for_current(core) {
                println!("{port}: Failed to set affinity to core {}", core.id);
            } else {
                println!("{port}: Setting affinity to CPU {}", core.id);
            }
            if set_current_thread_priority(thread_priority::ThreadPriority::Max).is_err() {
                println!(
                    "{port}: Warning: Could not set thread priority. Are you running as root?"
                );
            };

            listen_port(&address, port, barr, stat);
        }));
    }

    loop {
        state_rx.recv().unwrap();
        // thread::sleep(Duration::from_secs(20));
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
