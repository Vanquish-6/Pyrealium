//! Same-cell application replication only: excludes transport, sockets and disk.
use pyrealium::{
    movement::{ClientPosition, Placement},
    scene::{Body, Scene},
    world::{self, StationaryObject},
};
use std::{hint::black_box, time::Instant};

fn sample(mut operation: impl FnMut()) -> (u128, Vec<u128>) {
    for _ in 0..1000 {
        operation();
    }
    let mut samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        for _ in 0..10000 {
            operation();
        }
        samples.push(started.elapsed().as_nanos() / 10000);
    }
    samples.sort_unstable();
    (samples[2], samples)
}

fn main() {
    for players in [1, 16, 64] {
        let mut scene = Scene::default();
        let placement = Placement::HUMAN_START;
        for id in 1..=players {
            let spawn = world::create_stationary_object(&StationaryObject {
                character_id: id,
                parent_visual_did: 1,
                entity_did: 2,
                cell_id: placement.cell,
                position: placement.coordinates,
                orientation: placement.orientation(),
                instance_stamp: 1,
                visual_ordering_stamp: 0,
                timestamps: [0; 4],
            })
            .unwrap();
            black_box(
                scene
                    .enter(Body::new(id, placement, spawn).unwrap())
                    .unwrap(),
            );
        }
        let update = ClientPosition {
            time: 1.0,
            placement,
            motion: 0xbf7f7f7f,
            cancel_moveto: false,
            stamps: [1, 0, 0, 0],
            jump_velocity: None,
        };
        let (median, samples) = sample(|| {
            black_box(scene.update(1, 1, black_box(&update)).unwrap());
        });
        println!("players={players} movement_ns_median={median} samples_ns={samples:?}");
        let (median, samples) = sample(|| {
            black_box(scene.look(1, black_box(0.2), black_box(0.3)).unwrap());
        });
        println!("players={players} look_ns_median={median} samples_ns={samples:?}");
    }
}
