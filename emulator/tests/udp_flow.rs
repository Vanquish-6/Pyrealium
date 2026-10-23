//! Synthetic peer over real UDP. This does not substitute for running ac2client.exe.
use pyrealium::{
    server::{self, Config},
    store::Store,
    transport::Transport,
    wire::{self, Header},
    world,
};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

fn login(account: &str) -> Vec<u8> {
    let mut auth = Vec::new();
    for n in [1u32, 1, 1] {
        auth.extend(n.to_le_bytes());
    }
    auth.extend(wire::pack_string(account).unwrap());
    // Native Init supplies a 76-byte opaque CryptoAPI SIMPLEBLOB. This fixture
    // tests framing/policy, not decryption or Windows-generated key equivalence.
    auth.extend(76u32.to_le_bytes());
    auth.extend([0; 76]);
    auth.extend(0u32.to_le_bytes());
    let mut payload = wire::pack_string("7148").unwrap();
    payload.extend((auth.len() as u32).to_le_bytes());
    payload.extend(auth);
    wire::encode_handshake(
        Header {
            flags: wire::LOGON,
            ..Default::default()
        },
        &payload,
    )
    .unwrap()
}

#[test]
fn real_udp_login_ack_selection_enter_rejection_and_shutdown() {
    run_enter_rejection(None, false);
}

#[test]
#[ignore = "requires AC2_TEST_GAME_DATA pointing to the installed DAT directory"]
fn installed_dat_profile_rejects_7148_before_world_bootstrap() {
    run_enter_rejection(
        Some(std::env::var_os("AC2_TEST_GAME_DATA").unwrap().into()),
        false,
    );
}

#[test]
#[ignore = "requires AC2_TEST_COMPAT_DATA pointing to the converted test DAT directory"]
fn converted_dat_profile_sends_7148_world_bootstrap() {
    run_enter_rejection(
        Some(std::env::var_os("AC2_TEST_COMPAT_DATA").unwrap().into()),
        true,
    );
}

fn run_enter_rejection(game_data: Option<std::path::PathBuf>, expect_bootstrap: bool) {
    let profile_enabled = game_data.is_some();
    let (base, secondary) = (0..100)
        .find_map(|_| {
            let base = UdpSocket::bind("127.0.0.1:0").ok()?;
            let addr = base.local_addr().ok()?;
            let next = addr.port().checked_add(1)?;
            let secondary = UdpSocket::bind((addr.ip(), next)).ok()?;
            Some((base, secondary))
        })
        .unwrap();
    let addr = base.local_addr().unwrap();
    let ack_addr = secondary.local_addr().unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    drop(base);
    drop(secondary);
    let dir = std::env::temp_dir().join(format!("ac2-udp-{}-{}", std::process::id(), addr.port()));
    {
        let mut store = Store::open(&dir).unwrap();
        if profile_enabled {
            store
                .create_native(
                    "local",
                    &world::CharacterCreationRequest {
                        name: "Explorer".into(),
                        entity_did: 0x4700110f,
                        start_area: 0,
                        race: 1,
                        sex: 0x1000,
                        class: 0,
                        implement: 0,
                        physical_values: Vec::new(),
                    },
                )
                .unwrap();
        } else {
            store.create("local", "Explorer").unwrap();
        }
    }
    let server_dir = dir.clone();
    let worker = std::thread::spawn(move || {
        server::run(Config {
            bind: addr,
            account: "local".into(),
            data: server_dir,
            run_seconds: Some(2),
            visual_did: None,
            game_data,
        })
        .unwrap()
    });
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut buf = [0; 4096];
    let deadline = Instant::now() + Duration::from_secs(1);
    let (h, connect, connect_bytes) = loop {
        socket.send_to(&[0; 7], addr).unwrap(); // malformed input cannot kill the listener
        socket.send_to(&login("local"), addr).unwrap();
        match socket.recv_from(&mut buf) {
            Ok((len, source)) => {
                assert_eq!(source, addr);
                let (h, payload) = wire::decode_handshake(&buf[..len]).unwrap();
                break (
                    h,
                    wire::Connect::decode(payload).unwrap(),
                    buf[..len].to_vec(),
                );
            }
            Err(_) if Instant::now() < deadline => {
                // Windows may immediately report ICMP port-unreachable while
                // the server binds. Back off instead of flooding retry logons.
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(e) => panic!("server did not respond: {e}"),
        }
    };
    socket.send_to(&login("local"), addr).unwrap();
    let (len, _) = socket.recv_from(&mut buf).unwrap();
    assert_eq!(&buf[..len], connect_bytes); // repeated login preserves cookie/seeds
    let ack = wire::encode_handshake(
        Header {
            flags: wire::CONNECT_ACK,
            recipient: connect.net_id as u16,
            iteration: h.iteration,
            ..Default::default()
        },
        &connect.cookie.to_le_bytes(),
    )
    .unwrap();
    socket.send_to(&ack, ack_addr).unwrap();
    let mut peer = Transport::new(
        connect.outgoing_seed,
        connect.incoming_seed,
        h.recipient,
        h.iteration,
    );
    let ready = loop {
        let (len, source) = socket.recv_from(&mut buf).unwrap();
        assert_eq!(source, addr);
        if Header::decode(&buf[..len]).unwrap().flags == wire::CONNECT {
            continue;
        }
        break peer.receive(&buf[..len], Instant::now()).unwrap();
    };
    assert_eq!(ready.messages.len(), 1);
    assert_eq!(ready.messages[0].queue, 5);
    assert_eq!(ready.messages[0].blob_id >> 56, 0x23);
    assert_eq!(ready.messages[0].data, 0x1003bu32.to_le_bytes());
    let ack_ready = wire::encode_control_packet(
        Header {
            flags: wire::ACK,
            recipient: h.recipient,
            iteration: h.iteration,
            ..Default::default()
        },
        &ready.ack.to_le_bytes(),
        None,
    )
    .unwrap();
    socket.send_to(&ack_ready, addr).unwrap();
    let list = loop {
        let (len, source) = socket.recv_from(&mut buf).unwrap();
        assert_eq!(source, addr);
        if Header::decode(&buf[..len]).unwrap().flags == wire::CONNECT {
            continue;
        }
        break peer.receive(&buf[..len], Instant::now()).unwrap();
    };
    assert_eq!(list.messages.len(), 1);
    assert_eq!(list.messages[0].queue, 1);
    assert_eq!(list.messages[0].blob_id >> 56, 0x23);
    assert_eq!(
        list.messages[0].data,
        if profile_enabled {
            world::character_set(
                b"local",
                5,
                &[world::SelectionCharacter {
                    id: 1,
                    name: "Explorer".into(),
                    seconds_greyed_out: 0,
                    parent_visual_did: 0x1f000023,
                }],
            )
            .unwrap()
        } else {
            world::empty_character_set(b"local", 5).unwrap()
        }
    );
    let ack_list = wire::encode_control_packet(
        Header {
            flags: wire::ACK,
            recipient: h.recipient,
            iteration: h.iteration,
            ..Default::default()
        },
        &list.ack.to_le_bytes(),
        None,
    )
    .unwrap();
    socket.send_to(&ack_list, addr).unwrap();
    let event = world::encode_enter_world(1, b"local").unwrap();
    for packet in peer
        .send(0x0300000000000001, 4, &event, Instant::now())
        .unwrap()
    {
        socket.send_to(&packet, addr).unwrap();
    }
    let mut received_events = Vec::new();
    for _ in 0..32 {
        let (len, _) = socket.recv_from(&mut buf).unwrap();
        // A retried logon may already have queued an identical connection reply.
        if Header::decode(&buf[..len]).unwrap().flags == wire::CONNECT {
            continue;
        }
        let reply = peer.receive(&buf[..len], Instant::now()).unwrap();
        for message in reply.messages {
            received_events.push(message.data);
        }
        if received_events.len() == if expect_bootstrap { 3 } else { 1 } {
            break;
        }
    }
    if expect_bootstrap {
        assert_eq!(received_events.len(), 3);
        assert_eq!(received_events[0], world::create_player(1, 1));
        assert_eq!(
            &received_events[1][..4],
            &world::CREATE_OBJECT.to_le_bytes()
        );
        assert_eq!(&received_events[2][..4], &world::PLAYER_DESC.to_le_bytes());
        // Complete entry, leave, then enter again over the same encrypted UDP
        // session. The exit must use world stamp 2, not the global blob count.
        for packet in peer
            .send(
                0x0300000000000002,
                4,
                &world::SCENE_RENDERING_COMPLETE.to_le_bytes(),
                Instant::now(),
            )
            .unwrap()
        {
            socket.send_to(&packet, addr).unwrap();
        }
        let mut exit = world::CHAR_EXIT_GAME.to_le_bytes().to_vec();
        exit.extend(1u64.to_le_bytes());
        for packet in peer
            .send(0x0300000000000003, 4, &exit, Instant::now())
            .unwrap()
        {
            socket.send_to(&packet, addr).unwrap();
        }
        let mut found_exit = false;
        for _ in 0..32 {
            let (len, _) = socket.recv_from(&mut buf).unwrap();
            let reply = peer.receive(&buf[..len], Instant::now()).unwrap();
            for message in reply.messages {
                if message.data == world::char_exit_game() {
                    assert_eq!(message.blob_id >> 32, 0x01000002);
                    found_exit = true;
                }
            }
            if found_exit {
                break;
            }
        }
        assert!(found_exit, "server did not acknowledge leaving the world");
        for packet in peer
            .send(0x0300000000000004, 4, &event, Instant::now())
            .unwrap()
        {
            socket.send_to(&packet, addr).unwrap();
        }
        let mut reentry = Vec::new();
        for _ in 0..32 {
            let (len, _) = socket.recv_from(&mut buf).unwrap();
            let reply = peer.receive(&buf[..len], Instant::now()).unwrap();
            reentry.extend(reply.messages.into_iter().map(|message| message.data));
            if reentry.len() == 3 {
                break;
            }
        }
        assert_eq!(reentry, received_events);
    } else {
        assert_eq!(
            received_events,
            vec![world::character_error(
                world::CharacterError::CouldNotPlaceCharacter
            )]
        );
    }
    worker.join().unwrap();
    assert!(!dir.join("server.lock").exists());
    {
        let store = Store::open(&dir).unwrap();
        assert_eq!(store.characters[&1].name, "Explorer");
    }
    std::fs::remove_file(dir.join("characters.journal")).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
