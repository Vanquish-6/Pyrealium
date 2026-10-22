//! Build 7148 checksum masking, recovered from CryptoSystem/QTIsaac.
//! This is protocol compatibility, not a modern authentication primitive.

#[derive(Clone)]
pub struct Isaac {
    memory: [u32; 256],
    results: [u32; 256],
    a: u32,
    b: u32,
    c: u32,
    remaining: usize,
}

impl Isaac {
    pub fn new(seed: u32) -> Self {
        let mut state = Self {
            memory: [0; 256],
            results: [0; 256],
            a: seed,
            b: seed,
            c: seed,
            remaining: 0,
        };
        let mut mix = [0x9e37_79b9; 8];
        for _ in 0..4 {
            shuffle(&mut mix);
        }
        for pass in 0..2 {
            for base in (0..256).step_by(8) {
                for (j, value) in mix.iter_mut().enumerate() {
                    *value = value.wrapping_add(if pass == 0 {
                        state.results[base + j]
                    } else {
                        state.memory[base + j]
                    });
                }
                shuffle(&mut mix);
                state.memory[base..base + 8].copy_from_slice(&mix);
            }
        }
        state.generate();
        state
    }

    fn generate(&mut self) {
        self.c = self.c.wrapping_add(1);
        self.b = self.b.wrapping_add(self.c);
        for i in 0..256 {
            let x = self.memory[i];
            self.a ^= match i & 3 {
                0 => self.a << 13,
                1 => self.a >> 6,
                2 => self.a << 2,
                _ => self.a >> 16,
            };
            self.a = self.a.wrapping_add(self.memory[(i + 128) & 255]);
            let y = self.memory[((x >> 2) & 255) as usize]
                .wrapping_add(self.a)
                .wrapping_add(self.b);
            self.memory[i] = y;
            self.b = self.memory[((y >> 10) & 255) as usize].wrapping_add(x);
            self.results[i] = self.b;
        }
        self.remaining = 256;
    }

    /// Consume once per new encrypted packet; retain its key for retransmission.
    pub fn next_key(&mut self) -> u32 {
        if self.remaining == 0 {
            self.generate();
        }
        self.remaining -= 1;
        self.results[self.remaining]
    }
}

fn shuffle(v: &mut [u32; 8]) {
    v[0] ^= v[1] << 11;
    v[3] = v[3].wrapping_add(v[0]);
    v[1] = v[1].wrapping_add(v[2]);
    v[1] ^= v[2] >> 2;
    v[4] = v[4].wrapping_add(v[1]);
    v[2] = v[2].wrapping_add(v[3]);
    v[2] ^= v[3] << 8;
    v[5] = v[5].wrapping_add(v[2]);
    v[3] = v[3].wrapping_add(v[4]);
    v[3] ^= v[4] >> 16;
    v[6] = v[6].wrapping_add(v[3]);
    v[4] = v[4].wrapping_add(v[5]);
    v[4] ^= v[5] << 10;
    v[7] = v[7].wrapping_add(v[4]);
    v[5] = v[5].wrapping_add(v[6]);
    v[5] ^= v[6] >> 4;
    v[0] = v[0].wrapping_add(v[5]);
    v[6] = v[6].wrapping_add(v[7]);
    v[6] ^= v[7] << 8;
    v[1] = v[1].wrapping_add(v[6]);
    v[7] = v[7].wrapping_add(v[0]);
    v[7] ^= v[0] >> 9;
    v[2] = v[2].wrapping_add(v[7]);
    v[0] = v[0].wrapping_add(v[1]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_original_x86_execution_across_refill() {
        // Obtained by executing original 0065ff80/006603e0 in Unicorn,
        // independently of this Rust implementation. See tools/crypto_binary_vectors.py.
        let cases = [
            (
                0,
                [
                    0x182600f3, 0x300b4a8d, 0x301b6622, 0xb08acd21, 0xd91aa738, 0xe76dd339,
                    0x7a68710f, 0x6554abda, 0xf650e4c8,
                ],
            ),
            (
                1,
                [
                    0x38168fdd, 0x173ff3d7, 0x832ddc13, 0xc4291d70, 0xbd249809, 0xfe83b016,
                    0x1ba8a003, 0x061c754f, 0xa7b65ae0,
                ],
            ),
            (
                0x12345678,
                [
                    0xfdd4afeb, 0x398311d2, 0xc71829a4, 0xb19df96a, 0xd92733af, 0xdf7ca7e9,
                    0xa608787a, 0x2271e865, 0x4b91d57d,
                ],
            ),
            (
                u32::MAX,
                [
                    0x4317c641, 0x1d124727, 0xe877449d, 0x5c326457, 0xbd249809, 0xfe83b016,
                    0xe28dc1f7, 0x7b3c165a, 0xcd945ebf,
                ],
            ),
        ];
        let indices = [0, 1, 2, 3, 254, 255, 256, 257, 511];
        for (seed, expected) in cases {
            let mut generator = Isaac::new(seed);
            for i in 0..512 {
                let actual = generator.next_key();
                if let Some(j) = indices.iter().position(|&index| index == i) {
                    assert_eq!(actual, expected[j], "seed={seed:08x}, index={i}");
                }
            }
        }
    }
}
