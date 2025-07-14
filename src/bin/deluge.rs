use std::{
    io,
    iter::{self},
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Barrier},
    thread::{self},
    time::{Duration, Instant},
};

use bytemuck::{Zeroable, bytes_of};
use clap::Parser;
use itertools::multizip;
use morgul::{DelugeTrigger, SlsDetectorHeader, get_interface_addreses_with_prefix};
use socket2::Protocol;

#[derive(Parser, Debug)]
#[command(version, about, long_about=None)]
struct Args {
    /// The first target port to send data to
    #[arg(long, short, default_value = "30000")]
    target_port: u16,

    /// If set, only this many threads will be sent to the first target
    #[arg(long)]
    to_first: Option<u8>,
    target: Ipv4Addr,
    target_2: Option<Ipv4Addr>,
}

fn send_data(
    source_address: &Ipv4Addr,
    target_address: &Ipv4Addr,
    target_port: u16,
    sync: Arc<Barrier>,
    trigger: flume::Receiver<DelugeTrigger>,
) -> ! {
    let bind_addr: SocketAddr = format!("{source_address}:0").parse().unwrap();
    let to_addr: SocketAddr = format!("{target_address}:{target_port}").parse().unwrap();
    let socket = UdpSocket::bind(bind_addr).unwrap();
    let mut buff = vec![0u8; 8192 + size_of::<SlsDetectorHeader>()];
    let mut header = SlsDetectorHeader::zeroed();

    let mut last_send = Instant::now();
    loop {
        sync.wait();
        let acq = trigger.recv().unwrap();
        println!(
            "{target_port}: Starting {} images at {:.0} Hz",
            acq.frames, acq.exptime
        );
        for _ in 0..acq.frames {
            for _ in 0..64 {
                buff[..size_of::<SlsDetectorHeader>()].copy_from_slice(bytes_of(&header));

                let wait = acq.exptime - (Instant::now() - last_send).as_secs_f32();
                if wait > 0.0 {
                    thread::sleep(Duration::from_secs_f32(wait));
                }
                last_send = Instant::now();
                socket.send_to(&buff, to_addr).unwrap();
                header.packet_number += 1;
            }
            header.frame_number += 1;
            header.packet_number = 0;
        }
        println!("{target_port}: Sent {} images", acq.frames);
    }
}

pub fn new_reusable_udp_socket<T: std::net::ToSocketAddrs>(
    address: T,
) -> io::Result<std::net::UdpSocket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    let addr = address.to_socket_addrs()?.next().unwrap();
    socket.bind(&addr.into())?;
    Ok(socket.into())
}

fn main() {
    let args = Args::parse();

    println!("{args:?}");

    let interfaces = get_interface_addreses_with_prefix(192);
    if interfaces.is_empty() {
        println!("Error: Could not find any 192. interfaces. Have you set up the network?");
        std::process::exit(1);
    }
    // // Get a list of cores so that we can set affinity to them
    // let mut core_ids = core_affinity::get_core_ids().unwrap().into_iter().rev();
    // println!("{core_ids:?}");
    // println!("Start threads");

    let mut threads = Vec::new();
    // Work out the offset for target receivers
    let to_take: usize = (9 - args.to_first.unwrap_or(9)).into();

    let barrier = Arc::new(Barrier::new(interfaces.len() * 4));
    let (trigger_tx, trigger_rx) = flume::bounded(1);

    for (port, source, target) in multizip((
        args.target_port..(args.target_port + 8),
        interfaces.iter().flat_map(|x| iter::repeat_n(*x, 4)),
        iter::repeat_n(args.target, 9)
            .chain(iter::repeat_n(args.target_2.unwrap_or(args.target), 9))
            .skip(to_take),
    )) {
        println!("Starting {source} -> {target}:{port}");
        let bar = barrier.clone();
        let trig = trigger_rx.clone();
        threads.push(thread::spawn(move || {
            send_data(&source, &target, port, bar, trig);
        }));
    }
    drop(trigger_rx);
    // Wait for broadcasts
    let mut buf = vec![0; size_of::<DelugeTrigger>()];
    let broad = new_reusable_udp_socket("0.0.0.0:9999").unwrap();
    // let broad = UdpSocket::bind("0.0.0.0:9999").unwrap();
    // broad.recv(buf)
    // let mut last_trigger = None;
    loop {
        let size = broad.recv(buf.as_mut_slice()).unwrap();
        assert!(size == size_of::<DelugeTrigger>());
        let trigger: &DelugeTrigger = bytemuck::from_bytes(&buf);
        println!("Got trigger: {trigger:?}");

        trigger_tx.send(*trigger).unwrap();
        // last_trigger = Some(*trigger);
    }

    // loop {
    //     sleep(Duration::from_secs(5));
    //     println!("Sending Deluge");
    // }
}
