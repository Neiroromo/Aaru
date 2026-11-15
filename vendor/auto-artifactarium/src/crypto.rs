use std::collections::HashMap;

use rand_mt::Mt64;
use rayon::prelude::*;
use tracing::{debug, info, instrument, trace, warn};

use crate::bytes_as_hex;
use crate::cs_rand::Random;

#[instrument(skip_all)]
pub fn decrypt_command(key: &[u8], encrypted: &mut [u8]) {
    trace!(data = bytes_as_hex(encrypted), "before decryption");

    for i in 0..encrypted.len() {
        encrypted[i] ^= key[i % key.len()];
    }

    trace!(data = bytes_as_hex(encrypted), "after decryption");
}

pub fn lookup_initial_key(initial_keys: &HashMap<u16, Vec<u8>>, bytes: &[u8]) -> Option<Vec<u8>> {
    let version = u16::from_be_bytes(bytes[..2].try_into().unwrap()) ^ 0x4567;

    // attempt to fetch from user provided initial keys, otherwise use our own baked-in ones
    let key = initial_keys.get(&version).cloned();
    match key {
        Some(key) => {
            info!(version, "found initial decryption key");
            Some(key)
        }
        None => {
            info!(version, "didn't find decryption key");
            None
        }
    }
}

pub fn new_key_from_seed(seed: u64) -> Vec<u8> {
    // mersenne twister generator
    let mut first = Mt64::new(seed);
    let mut generator = Mt64::new(first.next_u64());

    let _ = generator.next_u64(); // skip first number

    let mut key = Vec::with_capacity(512);
    for _ in 0..512 {
        for b in generator.next_u64().to_be_bytes() {
            key.push(b);
        }
    }
    key
}

/// How far from the expected time the client seed is searched, per side.
///
/// The client seeds its random generator with its own clock, which only
/// roughly matches the time stamp of the request seen on the wire.
const SEARCH_WINDOW_MS: i64 = 30_000;

/// How many numbers of a freshly seeded generator are tried.
const FRESH_DEPTH: i32 = 5;

/// The client draws the seed of every connection from one generator that was
/// seeded at its first connection, so later connections use numbers further
/// down the sequence.  This is how far down is tried.
const RETAINED_DEPTH: i32 = 50_000;

/// A key found by searching, with where it was found.
pub struct FoundKey {
    /// The time the client's generator was seeded with.
    pub time: u64,
    /// Which number of that generator's sequence was used, counting from 1.
    pub output_index: usize,
    pub key: Vec<u8>,
}

/// The first eight bytes of the key for `seed`.  Enough to tell most wrong
/// seeds from the right one, at a fraction of the cost of the whole key.
fn key_head(seed: u64) -> [u8; 8] {
    let mut first = Mt64::new(seed);
    let mut generator = Mt64::new(first.next_u64());
    let _ = generator.next_u64(); // skip first number
    generator.next_u64().to_be_bytes()
}

fn is_command(bytes: &[u8]) -> bool {
    bytes[0] == 0x45
        && bytes[1] == 0x67
        && bytes[bytes.len() - 2] == 0x89
        && bytes[bytes.len() - 1] == 0xAB
}

/// Whether the number `client_seed` of a generator (with the seed of the server) gives the
/// key `data` was encrypted with.
fn fits(client_seed: u64, server_seed: u64, data: &[u8]) -> Option<Vec<u8>> {
    let seed = client_seed ^ server_seed;
    // Commands start with a known two byte magic.
    let head = key_head(seed);
    if data[0] ^ head[0] != 0x45 || data[1] ^ head[1] != 0x67 {
        return None;
    }
    let key = new_key_from_seed(seed);
    let mut clone = data.to_vec();
    decrypt_command(&key, &mut clone);
    if is_command(&clone) {
        debug!("Found encryption key seed: {seed}");
        Some(key)
    } else {
        None
    }
}

/// From this many numbers on, the numbers are tried on all the cores (before that, the
/// handing out of the work costs more than it saves).
const PARALLEL_FROM: i32 = 256;

/// Tries the first `depth` numbers of the generator seeded with `seed`.  The numbers
/// themselves come one from the other, but trying them does not: they are tried together, and
/// the first that fits (the nearest of the sequence) is the one taken.
fn guess(seed: i64, server_seed: u64, depth: i32, data: &[u8]) -> Option<(usize, Vec<u8>)> {
    if data.len() < 4 {
        return None;
    }

    let mut generator = Random::seeded(seed as i32);
    if depth < PARALLEL_FROM {
        for index in 1..=depth {
            if let Some(key) = fits(generator.next_safe_uint64(), server_seed, data) {
                return Some((index as usize, key));
            }
        }
        return None;
    }
    let numbers: Vec<u64> = (0..depth).map(|_| generator.next_safe_uint64()).collect();
    numbers
        .par_iter()
        .enumerate()
        .find_map_first(|(at, number)| fits(*number, server_seed, data).map(|key| (at + 1, key)))
}

/// Finds the key `data` was encrypted with by trying the generators seeded at
/// the times around `sent_time`, nearest first.  This is for the first
/// connection of a game session, whose generator is seeded about when it asks
/// for the token.
pub fn bruteforce(sent_time: u64, server_seed: u64, data: &[u8]) -> Option<FoundKey> {
    debug!("Running bruteforce loop.");
    // The times are tried together, on all the cores; the first that fits in the order of
    // nearest first is the one taken.
    let found = (0..=(2 * SEARCH_WINDOW_MS)).into_par_iter().find_map_first(|i| {
        let offset = if i % 2 == 0 { i / 2 } else { -(i - 1) / 2 };
        let time = sent_time as i64 + offset; // This will act as the seed.
        guess(time, server_seed, FRESH_DEPTH, data).map(|(output_index, key)| FoundKey {
            time: time as u64,
            output_index,
            key,
        })
    });
    if found.is_none() {
        warn!("Unable to find the encryption key seed.");
    }
    found
}

/// Finds the key when the client's generator is already known from an earlier
/// connection, and this one used a later number of its sequence.
pub fn retained_search(seed_time: u64, server_seed: u64, data: &[u8]) -> Option<FoundKey> {
    guess(seed_time as i64, server_seed, RETAINED_DEPTH, data).map(|(output_index, key)| FoundKey {
        time: seed_time,
        output_index,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encrypted_command(client_time: i64, nth_output: usize, server_seed: u64) -> Vec<u8> {
        let mut generator = Random::seeded(client_time as i32);
        let mut client_seed = 0;
        for _ in 0..nth_output {
            client_seed = generator.next_safe_uint64();
        }
        let key = new_key_from_seed(client_seed ^ server_seed);

        let mut command = vec![0x45, 0x67, 0, 1, 0, 4, 0, 0, 0, 6, 1, 2, 3, 4, 5, 6, 0x89, 0xAB];
        decrypt_command(&key, &mut command); // XOR, so this encrypts.
        command
    }

    #[test]
    fn finds_the_key_at_the_expected_time() {
        let command = encrypted_command(1_793_892_910_466, 1, 0x1234_5678_9abc_def0);
        let found = bruteforce(1_793_892_910_466, 0x1234_5678_9abc_def0, &command).unwrap();
        assert_eq!(found.time, 1_793_892_910_466);
        assert_eq!(found.output_index, 1);
    }

    #[test]
    fn finds_the_key_when_the_clocks_are_far_apart() {
        for (offset, nth) in [(-7_000, 3), (12_345, 5), (-29_999, 2)] {
            let client_time = 1_793_892_910_466 + offset;
            let command = encrypted_command(client_time, nth, 0xfeed_beef_0123_4567);
            let found = bruteforce(1_793_892_910_466, 0xfeed_beef_0123_4567, &command).unwrap();
            assert_eq!((found.time, found.output_index), (client_time as u64, nth));
        }
    }

    #[test]
    fn gives_up_outside_the_window() {
        let command = encrypted_command(1_793_892_910_466 + 45_000, 1, 0x1234_5678_9abc_def0);
        assert!(bruteforce(1_793_892_910_466, 0x1234_5678_9abc_def0, &command).is_none());
    }

    #[test]
    fn finds_a_later_connection_from_the_retained_seed_time() {
        // The seed time is long past, and this connection used a late number.
        let seed_time = 1_793_892_000_000;
        let command = encrypted_command(seed_time, 777, 0xabcd_ef01_2345_6789);
        assert!(bruteforce((seed_time + 900_000) as u64, 0xabcd_ef01_2345_6789, &command).is_none());
        let found = retained_search(seed_time as u64, 0xabcd_ef01_2345_6789, &command).unwrap();
        assert_eq!((found.time, found.output_index), (seed_time as u64, 777));
    }
}

#[cfg(test)]
mod speed {
    use super::*;

    /// How long the two searches take when nothing fits (the worst case); for looking at.
    #[test]
    #[ignore]
    fn the_worst_case_is_timed() {
        let data = vec![0u8; 64];
        let started = std::time::Instant::now();
        assert!(bruteforce(1_793_892_910_466, 0x1234_5678_9abc_def0, &data).is_none());
        println!("fresh search, nothing fits: {:?}", started.elapsed());
        let started = std::time::Instant::now();
        assert!(retained_search(1_793_892_000_000, 0x1234_5678_9abc_def0, &data).is_none());
        println!("retained search, nothing fits: {:?}", started.elapsed());
    }
}
