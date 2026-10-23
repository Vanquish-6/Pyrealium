# Build 7148 transport checksum masking

The Rust `crypto::Isaac` implementation is grounded in the supplied client binary, not an assumed AC1 algorithm. It has been checked against original x86 instructions executed offline, including a 256-word refill boundary. This establishes the PRNG arithmetic and consumption order, not live login acceptance.

## Evidence

All source paths below are relative to the distribution root. Recovered C is pseudocode; machine instructions resolve ambiguities.

| Behavior | Recovered symbol/address | Recovery file |
| --- | --- | --- |
| Seed copied into ISAAC a, b and c; lastIter starts at 1 | CryptoSystem::CryptoSystem, 00660390 | recovery/by-module/ac2client.exe/00785_CryptoSystem.obj.c |
| 256 zero result words, seed registers retained | QTIsaac constructor, 0065fee0 | same |
| Four golden-ratio mixes, two 256-word passes, first generation | randinit, 0065ff80 | same |
| 32-bit wraparound shuffle | shuffle, 00660280 | same |
| ISAAC generation, 256 words | isaac, 006603e0 | same |
| Outputs consumed 255 through 0, then refill | GetNextCryptoKey, 006606a0 | same |
| XOR one key over every complete 32-bit word; supplied key avoids advancement | EncryptData, 006606e0 | same |
| Incoming seed first, outgoing seed second | ReceiverData::InitCrypto, 004cb010 | recovery/by-module/ac2client.exe/00250_ReceiverData.obj.c |
| Retain keys for missing sequences/retransmits | ReceiverData::AddNakked, 004cb4b0 | same |
| Decrypt checksum when wire header bit 0x2 is set | SharedNet::ProcessNewSeqNum, 004c7c00 | recovery/by-module/ac2client.exe/00241_ClientNet.obj.c |
| Plain header checksum added after body checksum masking | SharedNet::SendPacket, 004c5640 | same |

The `shuffle` pseudocode misleadingly renders the last h update as though a is added twice. The original instructions at 00660336..00660345 add a exactly once then XOR a shifted right nine bits. Rust follows the instructions and the binary oracle.

The optional NetKeyExch object serializes a vlong and initialization flag (004cd860/004cd8b0). PortalDH::Init (0065fd30) only assigns shared base and prime. These functions alone do not establish an active DH exchange, nor provide its constants. The recovered connection path supplies two direct checksum seeds; no invented DH parameters are included in this implementation.

## Verification

Run `python tools/crypto_binary_vectors.py` from the emulator directory, with `pefile` and `unicorn` installed. It maps the original PE into an offline x86 emulator, supplies bounded private memory and a synthetic vtable, and executes the original `randinit` and `isaac` routines. It never launches the game process or opens a network connection. Those optional Python packages are verification tooling only; the Rust server has no dependency on them.

The Rust unit test checks 36 independent binary checkpoints: seeds 0, 1, 0x12345678, and 0xffffffff, at output indices 0, 1, 2, 3, 254, 255, 256, 257, and 511. A standalone test run passed on 2026-10-22. Integrating this module into the main crate makes the same oracle vectors part of `cargo test`.

## Integration obligations

Consume one key for each new encrypted packet in sequence order. Cache the key for retransmission and reserve keys for missing packets before consuming later ones. Do not consume a fresh key when retransmitting. Bound allowed sequence gaps before allocating key history or advancing the generator; invalid packets must not permanently desynchronize accepted session state. Wire header bit 0x2 is distinct from the internal NetPacket flags bit 0x1.

The initial reliable sequence is **2**, not 1: FlowQueue initializes `highestIDSent_` to 1 and increments it before encrypting a new reliable packet (00248_CFlowQueue.obj.c, SendDataEx). ReceiverData initializes `highestIDReceived_` to 1 in the connection path. Thus the first ISAAC key masks sequence 2. Plain control packets can carry the latest reliable sequence without consuming another key.

`transport.rs` implements a 512-packet receive window, transactional checksum validation, retained keys for reordering, duplicate suppression, 64 KiB message size, at most 16 in-progress assemblies, 30-second assembly expiration, at most 256 pending outgoing fragments, and eight retries with bounded backoff. Oversized sequence jumps are rejected before any key generation. These resource bounds are deliberate emulator policy rather than claims that the historical client used the same limits. Sequence wrap currently requires resetting the session. Completed ordered application messages on queues 3 and 7 share one 16-bit stamp counter, matching Proto_UI::SendBuff (004c9c20) and NetBlobIDUtils::GetOrderingStamp (0063a5b0). A bounded hold of 16 messages / 256 KiB delays later stamps until the missing earlier stamp arrives. The stamp is the low 16 bits of the blob ID high word; bits 16..23 belong to the blob sequence namespace. Stamp wrap currently requires resetting the session. Plain application fragments are rejected as local policy; plain control headers remain accepted. Queue 7 payload encryption is separate CryptoAPI behavior and is not implemented by checksum ISAAC.

Checksum masking provides legacy compatibility, not credential confidentiality or modern authenticated transport. Native-client auth, packet loss/reordering, and world entry still need matching-client acceptance runs. No live acceptance is claimed by these binary vectors.

## Control packet performance (2026-10-23)

The plain, sequence-zero control path now validates recipient, iteration, complete
packet structure and checksum before applying an acknowledgment. It does not clone
pending reliable payloads or consume ISAAC state. A future acknowledgment or corrupt
checksum leaves pending payloads unchanged. Encrypted/application packets retain the
transactional candidate-state path. Acknowledgments retire only the acknowledged
prefix of the pending map; duplicate acknowledgments do not traverse the whole map.

`cargo bench --bench transport` measures a non-advancing valid ACK with 0, 16 and
64 outstanding 1-KiB messages (the latter occupy 192 fragments). Five samples of
10,000 receives follow warm-up, with no sockets, sleeps or file I/O inside samples.
On this host, median cost before/after was 113/32 ns with zero messages, 2,562/31 ns
with 16, and 10,230/31 ns with 64. These are controlled microbenchmarks of duplicate
ACK processing, not end-to-end player latency or a general server throughput claim.
Tests cover corrupt/future acknowledgments, exact pending retirement, duplicate ACKs,
and unchanged receive sequence/key state.

## Shared immutable retry storage (2026-10-23)

Pending datagram bytes now use `Arc<[u8]>`. The transactional encrypted receive
path still clones independent metadata and commits only after validation, but
does not copy every outstanding retry payload. Retry methods materialize output
buffers from the unchanged stored bytes when retransmission is actually due.

The expanded `benches/transport.rs` measures ACKs, warmed encrypted duplicates
and fresh ordered encrypted messages. Fresh-input generation and receiver setup
are outside the samples. Five samples each process 10,000 inputs; message bodies
are 48 bytes and outstanding output messages are 1 KiB each. Before/after median
nanoseconds on the same development host:

- Zero outstanding: duplicate 208 / 230; fresh 5,829 / 6,070.
- Sixteen outstanding: duplicate 2,823 / 1,225; fresh 8,840 / 7,144.
- Sixty-four outstanding: duplicate 10,120 / 4,485; fresh 16,205 / 10,482.

ACKs remained 31–32 ns in these runs. The optimization removes retry-byte copying
when output is queued; it does not establish a speedup with no pending output.
Fresh receives still copy bounded assembly/order/checksum metadata. These are
isolated transport costs, excluding socket I/O, world processing and persistence.
Raw samples: `target/performance/encrypted-before.txt` and `encrypted-after.txt`.

An added regression combines a checksum-valid ACK with conflicting fragment
counts. Rejection must preserve the original shared retry bytes, pending count,
assembly state and ISAAC state. The following valid packet must still decode,
and a timed retry must equal the original datagram byte for byte.

