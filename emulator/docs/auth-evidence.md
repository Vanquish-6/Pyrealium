# AC2 7148 transport and authentication evidence

All paths below are relative to `recovery/exports/ac2client.exe/`. These are recovered client pseudocode, not server source or live acceptance evidence.

| Contract | Evidence |
| --- | --- |
| 20-byte little-endian packet header: sequence u32, flags u32, checksum u32, recipient u16, interval u16, payload length u16, iteration u16 | `recovered-types.h`, `struct ProtoHeader`; `functions/004c5440.c` |
| Header checksum uses `0xbadd70dd` in place of checksum field | `functions/004c5440.c` SharedNet::ChecksumHeader |
| Body checksum is added to header checksum | `functions/004c5640.c` SharedNet::SendPacket |
| Checksum sums little-endian words plus length shifted 16, then remaining bytes shifted 24,16,8 | `functions/0065f3d0.c` PortalChecksum::CalcChecksum32 |
| Optional headers and fragment header/body each contribute separate checksum segments | `functions/004cb7d0.c` NetPacket::ComputeChecksum |
| Initial login uses sequence/recipient/interval/iteration zero, version `7148`, mask `0x10000` | `functions/004c82c0.c`; `functions/0063c270.c` |
| Login body: packed version, u32 authenticator byte count, authenticator | `functions/0063c340.c` |
| Authenticator: auth type u32, flags u32, connection sequence u32, packed account, optional override account if flags bit2 set, length-prefixed crypto bytes, length-prefixed extra bytes | `functions/004c5200.c`; `functions/004c4f80.c` |
| Packed strings: u16 length (0xffff escape plus u32), bytes, 4-byte alignment; encode nibble swaps then XORs complete u32 blocks with a repeating 16-word table | `functions/004042f0.c`; `functions/00403030.c`; `functions/00401fc0.c`; inverse `functions/00406cb0.c` |
| Connect mask `0x40000`, 32-byte native structure | `functions/0063bbe0.c`; `recovered-types.h` CConnectHeader and __unnamed_00005AA4 |
| Connect body fields f64 server time, u64 cookie, u32 NetID, u32 outgoing seed, u32 incoming seed; 4 trailing padding bytes | Same connect type and raw 32-byte copy |
| Client binds incoming generator to connect outgoing seed and outgoing generator to connect incoming seed | `functions/004c9200.c`; `functions/004cb010.c` |
| Connect ACK mask `0x80000`, eight-byte cookie; critically sent to source server port plus one | `functions/004c7630.c` ClientNet::SendConnectAck |
| Blob fragment header: u64 ID, u16 total fragment count, u16 size including header, u16 fragment index, u16 queue ID | `recovered-types.h` BlobFragHeader_t; `functions/004cbda0.c` |
| Checksum masking uses ISAAC output; encrypted checksum mode requires nonzero sequence | `functions/004cbe60.c`; `functions/00660390.c`; `functions/006606e0.c`; `functions/004c7d00.c` |

The handshake decoder rejects unknown/combined handshake flags. The session decoder supports ACK `0x4000`, NAK `0x1000`, empty ACK list `0x2000`, time sync `0x1000000`, echo request/response `0x2000000`/`0x4000000`, six-byte flow `0x8000000`, multiple fragments, and checksum masking with a supplied ISAAC key. Optional headers sort by ascending mask (`functions/0063b7e0.c`, `functions/0063b880.c`); their sizes are grounded in `0063bd00`, `0063be40`, `0063bde0`, `0063ba70`, `0063baf0`, `0063bb50`, and `0063bc60`. Reassembly, replay/order handling, retries and generator progression belong above the codec. UTF-8 account parsing is deliberately narrower than the recovered client's arbitrary narrow strings. Parsing an account is not identity verification; any local-development authentication policy must be explicit. Fixed string obfuscation is protocol encoding, not a credential protection mechanism.

Validation: independently compiled `wire.rs` unit tests cover exact checksum tails/wrapping, the recovered `7148` string vector, every truncated prefix of valid handshake/login/fragment samples, wrong checksums, extra bytes, invalid fragment indices and oversized auth lengths. Native client handshake and world acceptance still require live testing.
# Native connection test, 2026-10-22

The recovered 7148 executable successfully started with `C:\Asheron's Call 2`
as its working directory and `--rodat`. That installation's own executable is
7230; it was not replaced. The 7148 client rendered the connection screen and
sent logon datagrams to Pyrealium on UDP 19070.

Native logon observations: 136-byte datagram, mask 0x10000, sequence 0, build
7148, auth type 1, auth flags 1, matching local account, 76 crypto bytes, zero
extra bytes. The original no-crypto synthetic peer omitted this normal path.
`ClientNet::Init` (004c9050) calls `CryptExportKey` with blob type 1, then
`NetAuthenticator::SetCryptoKey` (004c4f10) sets auth flag bit 0. The local
identity policy now accepts the bounded opaque blob. This is not key decryption
or external identity verification; queue-7 payload cryptography remains absent.

The UDP regression fixture now includes the flag and 76-byte opaque blob.
Character-selection acceptance must be checked separately from receiving logon.

After the policy correction, a fresh native attempt displayed **Connection
successful!** in the 7148 client. The running server recorded one login, one
completed connection, and 26 received packets with zero rejections at the
observation boundary. The client remained on the connection artwork; character
selection and world entry were not established. This verifies the native local
handshake, not full authentication services or gameplay.

