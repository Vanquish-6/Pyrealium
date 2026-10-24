//! Two synthetic native-protocol peers over UDP; no native-rendering claim.
use pyrealium::{
    movement::{ClientPosition, Placement},
    server::{self, Config},
    store::Store,
    transport::Transport,
    wire::{self, Header},
    world,
};
use std::{
    collections::VecDeque,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

struct Peer {
    socket: UdpSocket,
    addr: SocketAddr,
    header: Header,
    transport: Transport,
    messages: VecDeque<Vec<u8>>,
    blob: u32,
    ordered: u16,
}
impl Peer {
    fn connect(addr: SocketAddr, socket: UdpSocket) -> Self {
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut auth = Vec::new();
        for n in [1u32, 1, 1] {
            auth.extend(n.to_le_bytes());
        }
        auth.extend(wire::pack_string("local").unwrap());
        auth.extend(76u32.to_le_bytes());
        auth.extend([0; 76]);
        auth.extend(0u32.to_le_bytes());
        let mut payload = wire::pack_string("7148").unwrap();
        payload.extend((auth.len() as u32).to_le_bytes());
        payload.extend(auth);
        let logon = wire::encode_handshake(
            Header {
                flags: wire::LOGON,
                ..Default::default()
            },
            &payload,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut bytes = [0; 4096];
        let (header, connection) = loop {
            socket.send_to(&logon, addr).unwrap();
            if let Ok((size, _)) = socket.recv_from(&mut bytes) {
                let (header, payload) = wire::decode_handshake(&bytes[..size]).unwrap();
                break (header, wire::Connect::decode(payload).unwrap());
            }
            assert!(Instant::now() < deadline, "server did not connect");
            std::thread::sleep(Duration::from_millis(10));
        };
        let ack = wire::encode_handshake(
            Header {
                flags: wire::CONNECT_ACK,
                recipient: header.recipient,
                iteration: header.iteration,
                ..Default::default()
            },
            &connection.cookie.to_le_bytes(),
        )
        .unwrap();
        socket
            .send_to(&ack, SocketAddr::new(addr.ip(), addr.port() + 1))
            .unwrap();
        let mut peer = Self {
            socket,
            addr,
            header,
            transport: Transport::new(
                connection.outgoing_seed,
                connection.incoming_seed,
                header.recipient,
                header.iteration,
            ),
            messages: VecDeque::new(),
            blob: 0,
            ordered: 0,
        };
        assert_eq!(peer.events(1)[0], 0x1003bu32.to_le_bytes());
        assert_eq!(&peer.events(1)[0][..4], &world::CHARACTER_SET.to_le_bytes());
        peer
    }
    fn send(&mut self, queue: u16, payload: &[u8]) {
        self.blob += 1;
        let namespace = if queue == 3 {
            self.ordered += 1;
            0x01000000 | u32::from(self.ordered)
        } else {
            0x03000000 | self.blob
        };
        let id = u64::from(namespace) << 32 | u64::from(self.blob);
        for packet in self
            .transport
            .send(id, queue, payload, Instant::now())
            .unwrap()
        {
            self.socket.send_to(&packet, self.addr).unwrap();
        }
    }
    fn events(&mut self, count: usize) -> Vec<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut bytes = [0; 4096];
        while self.messages.len() < count {
            assert!(
                Instant::now() < deadline,
                "expected {count} messages, got {}",
                self.messages.len()
            );
            let Ok((size, _)) = self.socket.recv_from(&mut bytes) else {
                continue;
            };
            let header = Header::decode(&bytes[..size]).unwrap();
            if header.flags == wire::CONNECT {
                continue;
            }
            let received = self
                .transport
                .receive(&bytes[..size], Instant::now())
                .unwrap();
            if header.sequence != 0 {
                let ack = wire::encode_control_packet(
                    Header {
                        flags: wire::ACK,
                        recipient: self.header.recipient,
                        iteration: self.header.iteration,
                        ..Default::default()
                    },
                    &received.ack.to_le_bytes(),
                    None,
                )
                .unwrap();
                self.socket.send_to(&ack, self.addr).unwrap();
            }
            self.messages
                .extend(received.messages.into_iter().map(|m| m.data));
        }
        self.messages.drain(..count).collect()
    }
    fn enter(&mut self, id: u64) {
        self.send(4, &world::encode_enter_world(id, b"local").unwrap());
    }
}

#[test]
#[ignore = "requires AC2_TEST_COMPAT_DATA pointing to the converted build-7148 DAT directory"]
fn two_peers_see_movement_look_departure_and_saved_reentry() {
    let data = std::env::var_os("AC2_TEST_COMPAT_DATA").expect("AC2_TEST_COMPAT_DATA");
    let (base, secondary) = (0..100)
        .find_map(|_| {
            let a = UdpSocket::bind("127.0.0.1:0").ok()?;
            let addr = a.local_addr().ok()?;
            let b = UdpSocket::bind((addr.ip(), addr.port().checked_add(1)?)).ok()?;
            Some((a, b))
        })
        .unwrap();
    let addr = base.local_addr().unwrap();
    let a_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let b_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    drop(base);
    drop(secondary);
    let dir = std::env::temp_dir().join(format!(
        "ac2-two-peers-{}-{}",
        std::process::id(),
        addr.port()
    ));
    {
        let mut store = Store::open(&dir).unwrap();
        for name in ["Aster", "Birch"] {
            store
                .create_native(
                    "local",
                    &world::CharacterCreationRequest {
                        name: name.into(),
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
        }
    }
    let server_dir = dir.clone();
    let worker = std::thread::spawn(move || {
        server::run(Config {
            initialize_ui: false,
            bind: addr,
            account: "local".into(),
            data: server_dir,
            run_seconds: Some(15),
            visual_did: None,
            game_data: Some(data.into()),
        })
        .unwrap()
    });
    let mut a = Peer::connect(addr, a_socket);
    a.enter(1);
    assert_eq!(a.events(3)[0], world::create_player(1, 1));
    a.send(4, &world::SCENE_RENDERING_COMPLETE.to_le_bytes());
    let mut b = Peer::connect(addr, b_socket);
    // A character already in use cannot be taken over by another peer.
    b.enter(1);
    b.enter(2);
    let spawned = b.events(3);
    assert_eq!(spawned[0], world::create_player(2, 1));
    // Gameplay requests while the client is still loading cannot change state.
    b.send(
        3,
        &pyrealium::script::client_call(pyrealium::script::COMBAT_START, &[]).unwrap(),
    );
    b.send(4, &world::SCENE_RENDERING_COMPLETE.to_le_bytes());
    assert_eq!(&b.events(1)[0][4..12], &1u64.to_le_bytes());
    assert_eq!(&a.events(1)[0][4..12], &2u64.to_le_bytes());
    // Script actions share the same ordered queues as movement. A forbidden
    // target and malformed combat request must not block the next valid action.
    use pyrealium::script;
    a.send(
        3,
        &script::client_call(script::TARGET, &script::identity(999)).unwrap(),
    );
    a.send(
        3,
        &script::client_call(script::TARGET, &script::identity(2)).unwrap(),
    );
    a.send(3, &script::client_call(script::COMBAT_START, &[1]).unwrap());
    a.send(3, &script::client_call(script::COMBAT_START, &[]).unwrap());
    assert_eq!(
        a.events(1)[0],
        script::private_call(script::COMBAT_STATE, &script::integer(1)).unwrap()
    );
    a.send(3, &script::client_call(script::COMBAT_STOP, &[]).unwrap());
    assert_eq!(
        a.events(1)[0],
        script::private_call(script::COMBAT_STATE, &script::integer(0)).unwrap()
    );
    let mut say = script::integer(0);
    say.extend(script::literal("Hello world").unwrap());
    a.send(3, &script::client_call(script::SAY, &say).unwrap());
    let heard = a.events(1).remove(0);
    assert_eq!(heard, b.events(1).remove(0));
    assert_eq!(&heard[..4], &0x10066u32.to_le_bytes());
    assert_eq!(&heard[4..8], &script::CHAT_STRING.to_le_bytes());
    let mut tell = script::integer(0);
    tell.extend(script::literal("A private reply").unwrap());
    tell.extend(script::literal("Aster").unwrap());
    b.send(3, &script::client_call(script::TELL, &tell).unwrap());
    let private_reply = a.events(1).remove(0);
    assert_eq!(&private_reply[4..8], &script::CHAT_STRING.to_le_bytes());
    assert_eq!(&b.events(1)[0][4..8], &script::CHAT_STRING.to_le_bytes());
    let mut placement = Placement::HUMAN_START;
    placement.coordinates[0] += 4.0;
    let update = ClientPosition {
        time: 10.0,
        placement,
        motion: 0xbf7f7f7f,
        cancel_moveto: false,
        stamps: [1, 0, 0, 0],
        jump_velocity: None,
    };
    a.send(3, &update.encode().unwrap());
    let moved = b.events(1).remove(0);
    assert_eq!(&moved[..4], &0x10086u32.to_le_bytes());
    assert_eq!(&moved[4..12], &1u64.to_le_bytes());
    assert_eq!(&moved[28..32], &placement.coordinates[0].to_le_bytes());
    let mut look = 0x100c8u32.to_le_bytes().to_vec();
    look.extend(0.25f32.to_le_bytes());
    look.extend(0.5f32.to_le_bytes());
    a.send(3, &look);
    let looked = b.events(1).remove(0);
    assert_eq!(&looked[..4], &0x100c9u32.to_le_bytes());
    assert_eq!(&looked[16..20], &0.25f32.to_le_bytes());
    let mut target_look = 0x100c7u32.to_le_bytes().to_vec();
    // Invisible/unknown targets must not emit a reply before the valid one.
    target_look.extend(999u64.to_le_bytes());
    a.send(3, &target_look);
    target_look[4..12].copy_from_slice(&2u64.to_le_bytes());
    a.send(3, &target_look);
    let looked_at = b.events(1).remove(0);
    assert_eq!(looked_at.len(), 24);
    assert_eq!(&looked_at[..4], &0x10096u32.to_le_bytes());
    assert_eq!(&looked_at[4..12], &1u64.to_le_bytes());
    assert_eq!(&looked_at[16..24], &2u64.to_le_bytes());
    let mut logout = world::CHAR_EXIT_GAME.to_le_bytes().to_vec();
    logout.extend(1u64.to_le_bytes());
    a.send(4, &logout);
    assert_eq!(a.events(2)[0], world::char_exit_game());
    assert_eq!(&b.events(1)[0][..4], &0x100c2u32.to_le_bytes());
    a.enter(1);
    let reentered = a.events(3);
    assert_eq!(
        &reentered[1][32..36],
        &placement.coordinates[0].to_le_bytes()
    );
    a.send(4, &world::SCENE_RENDERING_COMPLETE.to_le_bytes());
    assert_eq!(&a.events(1)[0][4..12], &2u64.to_le_bytes());
    assert_eq!(&b.events(1)[0][24..26], &2u16.to_le_bytes());
    // A stale connection identity cannot release this peer's character.
    let logoff_header = Header {
        flags: wire::LOGOFF,
        recipient: b.header.recipient,
        iteration: b.header.iteration,
        ..Default::default()
    };
    let stale_logoff = wire::encode_control_packet(
        Header {
            iteration: logoff_header.iteration.wrapping_add(1),
            ..logoff_header
        },
        &[],
        None,
    )
    .unwrap();
    b.socket.send_to(&stale_logoff, b.addr).unwrap();
    a.send(3, &look);
    assert_eq!(&b.events(1)[0][..4], &0x100c9u32.to_le_bytes());
    let logoff = wire::encode_control_packet(logoff_header, &[], None).unwrap();
    b.socket.send_to(&logoff, b.addr).unwrap();
    let departure = a.events(1).remove(0);
    assert_eq!(&departure[..4], &0x100c2u32.to_le_bytes());
    assert_eq!(&departure[4..12], &2u64.to_le_bytes());
    // Deletion is selection-only; an invalid request must not break later logout.
    a.send(4, &world::encode_delete_character(1, b"local").unwrap());
    a.send(4, &logout);
    assert_eq!(a.events(2)[0], world::char_exit_game());
    a.send(4, &world::encode_delete_character(1, b"other").unwrap());
    a.send(4, &world::encode_delete_character(1, b"local").unwrap());
    assert_eq!(a.events(1)[0], world::character_deleted(1));
    a.enter(1);
    assert_eq!(
        a.events(1)[0],
        world::character_error(world::CharacterError::CharacterNotOwned)
    );
    // The cleanly disconnected character is immediately available to another
    // connection, without waiting for the server's idle expiry.
    a.enter(2);
    let takeover = a.events(3);
    assert_eq!(takeover[0], world::create_player(2, 1));
    assert_eq!(&takeover[1][24..26], &2u16.to_le_bytes());
    std::fs::write(dir.join("stop.request"), b"stop").unwrap();
    worker.join().unwrap();
    {
        let mut store = Store::open(&dir).unwrap();
        assert_eq!(store.placement(1), Some(placement));
        assert!(store.characters[&1].deleted);
        store.restore("local", 1).unwrap();
        assert!(!store.characters[&1].deleted);
        assert_eq!(store.begin_instance("local", 1).unwrap(), 3);
        assert_eq!(store.begin_instance("local", 2).unwrap(), 3);
    }
    for file in ["characters.journal", "positions.snapshot"] {
        std::fs::remove_file(dir.join(file)).unwrap();
    }
    std::fs::remove_dir(dir).unwrap();
}
