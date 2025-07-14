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

    /// The port to listen for broadcast triggers on
    #[arg(default_value = "9999", long)]
    trigger_port: u16,
}

fn send_data(
    source_address: &Ipv4Addr,
    target_address: &Ipv4Addr,
    target_port: u16,
    sync: Arc<Barrier>,
    mut trigger: bus::BusReader<DelugeTrigger>,
) -> ! {
    let bind_addr: SocketAddr = format!("{source_address}:0").parse().unwrap();
    let to_addr: SocketAddr = format!("{target_address}:{target_port}").parse().unwrap();
    let socket = UdpSocket::bind(bind_addr).unwrap();
    let mut buff = vec![0u8; 8192 + size_of::<SlsDetectorHeader>()];
    let mut header = SlsDetectorHeader::zeroed();

    sync.wait();
    loop {
        let acq = trigger.recv().unwrap();
        println!(
            "{target_port}: Starting {} images at {:.0} Hz",
            acq.frames,
            1.0 / acq.exptime
        );
        // println!("{target_port}: Starting send");
        let start_acq = Instant::now();
        for image_num in 0..2000 {
            let acq_elapsed = (Instant::now() - start_acq).as_secs_f32();
            if acq_elapsed < image_num as f32 * acq.exptime {
                thread::sleep(Duration::from_secs_f32(
                    image_num as f32 * acq.exptime - acq_elapsed,
                ));
            }
            for _ in 0..64 {
                buff[..size_of::<SlsDetectorHeader>()].copy_from_slice(bytes_of(&header));

                socket.send_to(&buff, to_addr).unwrap();
                header.packet_number += 1;
            }

            header.frame_number += 1;
            header.packet_number = 0;
        }
        println!("{target_port}: Sent {} images", acq.frames);
        let sync_result = sync.wait();
        if sync_result.is_leader() {
            println!(
                "Sent 2000 images in {:.0}",
                (Instant::now() - start_acq).as_millis()
            );
        }
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
    // socket.set_nonblocking(true)?;
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
    let mut bus = bus::Bus::new(1);

    for (port, source, target) in multizip((
        args.target_port..(args.target_port + interfaces.len() as u16 * 4),
        interfaces.iter().flat_map(|x| iter::repeat_n(*x, 4)),
        iter::repeat_n(args.target, 9)
            .chain(iter::repeat_n(args.target_2.unwrap_or(args.target), 9))
            .cycle()
            .skip(to_take),
    )) {
        println!("Starting {source} -> {target}:{port}");
        let bar = barrier.clone();
        let trig = bus.add_rx();
        threads.push(thread::spawn(move || {
            send_data(&source, &target, port, bar, trig);
        }));
    }

    // drop(trigger_rx);
    // Wait for broadcasts
    let mut buf = vec![0; size_of::<DelugeTrigger>()];
    let broad = new_reusable_udp_socket("0.0.0.0:9999").unwrap();
    // let broad = UdpSocket::bind("0.0.0.0:9999").unwrap();
    // broad.recv(buf)
    // let mut last_trigger = None;
    let mut last_trigger = None;
    loop {
        if let Ok(size) = broad.recv(buf.as_mut_slice()) {
            assert!(size == size_of::<DelugeTrigger>());
            let trigger: &DelugeTrigger = bytemuck::from_bytes(&buf);
            // Ignore retriggers within 0.5 s
            if let Some(last) = last_trigger
                && (Instant::now() - last) < Duration::from_millis(500)
            {
                last_trigger = Some(Instant::now());
                continue;
            }

            bus.broadcast(*trigger);
            // trigger_tx.send(*trigger).unwrap();

            // println!("Rec: {:?}", trigger_rx.receiver_count());
            // println!(" Getting: {:?}", trigger_rx.recv());
            // println!(" Barrier: {:?}", barrier.)
            barrier.wait();
            last_trigger = Some(Instant::now());
        }
    }
}
