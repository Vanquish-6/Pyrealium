//! Reproducible ACK and encrypted-duplicate cost with outstanding reliable payloads.
//! Run: cargo bench --bench transport. No sockets, sleeps, or disk I/O in samples.
use pyrealium::{
    transport::Transport,
    wire::{self, Header},
};
use std::{hint::black_box, time::Instant};

fn main() {
    for outstanding_messages in [0, 16, 64] {
        let mut transport = Transport::new(1, 2, 3, 4);
        let now = Instant::now();
        for id in 1..=outstanding_messages {
            black_box(transport.send(id, 1, &[0x5a; 1024], now).unwrap());
        }
        let packet = wire::encode_control_packet(
            Header {
                flags: wire::ACK,
                recipient: 3,
                iteration: 4,
                ..Default::default()
            },
            &1u32.to_le_bytes(),
            None,
        )
        .unwrap();
        for _ in 0..1000 {
            black_box(transport.receive(black_box(&packet), now).unwrap());
        }
        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            for _ in 0..10000 {
                black_box(transport.receive(black_box(&packet), now).unwrap());
            }
            samples.push(started.elapsed().as_nanos() / 10000);
        }
        samples.sort_unstable();
        println!(
            "outstanding_messages={outstanding_messages} ack_ns_median={} samples_ns={samples:?}",
            samples[2]
        );
        let mut sender = Transport::new(2, 1, 3, 4);
        let encrypted = sender
            .send(0x0100000100000001, 3, &[0x42; 48], now)
            .unwrap()
            .remove(0);
        for _ in 0..1000 {
            black_box(transport.receive(black_box(&encrypted), now).unwrap());
        }
        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            for _ in 0..10000 {
                black_box(transport.receive(black_box(&encrypted), now).unwrap());
            }
            samples.push(started.elapsed().as_nanos() / 10000);
        }
        samples.sort_unstable();
        println!(
            "outstanding_messages={outstanding_messages} encrypted_duplicate_ns_median={} samples_ns={samples:?}",
            samples[2]
        );
        // Build fresh encrypted inputs outside the samples, retiring only the
        // fixture sender's queue. Each sample starts from the same receiver.
        let packets: Vec<_> = (2..10002u64)
            .map(|index| {
                let packet = sender
                    .send(((0x01000000 + index) << 32) | index, 3, &[0x42; 48], now)
                    .unwrap()
                    .remove(0);
                sender
                    .acknowledge(Header::decode(&packet).unwrap().sequence)
                    .unwrap();
                packet
            })
            .collect();
        let mut samples = Vec::new();
        for _ in 0..5 {
            let mut receiver = transport.clone();
            let started = Instant::now();
            for packet in &packets {
                black_box(receiver.receive(black_box(packet), now).unwrap());
            }
            samples.push(started.elapsed().as_nanos() / packets.len() as u128);
        }
        samples.sort_unstable();
        println!(
            "outstanding_messages={outstanding_messages} fresh_application_ns_median={} samples_ns={samples:?}",
            samples[2]
        );
    }
}
