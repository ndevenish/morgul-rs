use std::{
    iter::{self},
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Barrier},
    thread::{self},
    time::{Duration, Instant},
};

use bytemuck::{Zeroable, bytes_of};
use clap::Parser;
use itertools::multizip;
use morgul::{SlsDetectorHeader, get_interface_addreses_with_prefix};

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
) -> ! {
    let bind_addr: SocketAddr = format!("{source_address}:0").parse().unwrap();
    let to_addr: SocketAddr = format!("{target_address}:{target_port}").parse().unwrap();
    let socket = UdpSocket::bind(bind_addr).unwrap();
    let mut buff = vec![0u8; 8192 + size_of::<SlsDetectorHeader>()];
    let mut header = SlsDetectorHeader::zeroed();

    // let mut last_send = Instant::now();
    let exp_time = 0.0005;
    loop {
        let sync_result = sync.wait();
        // println!("{target_port}: Starting send");
        let start_acq = Instant::now();
        for image_num in 0..2000 {
            let acq_elapsed = (Instant::now() - start_acq).as_secs_f32();
            if acq_elapsed < image_num as f32 * exp_time {
                thread::sleep(Duration::from_secs_f32(
                    image_num as f32 * exp_time - acq_elapsed,
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
        if sync_result.is_leader() {
            println!(
                "Sent 2000 images in {:.0}",
                (Instant::now() - start_acq).as_millis()
            );
        }
    }
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

    println!("To take: {to_take}");

    let barrier = Arc::new(Barrier::new(interfaces.len() * 4 + 1));

    for (port, source, target) in multizip((
        args.target_port..(args.target_port + 8),
        interfaces.iter().flat_map(|x| iter::repeat_n(*x, 4)),
        iter::repeat_n(args.target, 9)
            .chain(iter::repeat_n(args.target_2.unwrap_or(args.target), 9))
            .skip(to_take),
    )) {
        println!("Starting {source} -> {target}:{port}");
        let bar = barrier.clone();
        threads.push(thread::spawn(move || {
            send_data(&source, &target, port, bar);
        }));
    }
    barrier.wait();
    loop {
        barrier.wait();
        // sleep(Duration::from_secs(5));
        // println!("Sending Deluge");
    }
}
