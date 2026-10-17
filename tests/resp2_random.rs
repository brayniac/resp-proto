use bytes::Bytes;
use resp_proto::{Frame, ParseError, ParseOptions, Value};
use std::time::Instant;

const ITERATIONS: usize = 100_000;
const MAX_INPUT_LEN: usize = 24;

#[derive(Default, Debug)]
struct Coverage {
    success: usize,
    incomplete: usize,
    error: usize,
    trailing: usize,
    roundtrip: usize,
    random: usize,
}

fn next_u64(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn input_for(iteration: usize, state: &mut u64) -> (Vec<u8>, bool) {
    let fixture = match iteration % 16 {
        0 => Some(&b"+OK\r\n"[..]),
        1 => Some(&b"-ERR test\r\n"[..]),
        2 => Some(&b":42\r\n"[..]),
        3 => Some(&b"$3\r\nfoo\r\n"[..]),
        4 => Some(&b"*0\r\n"[..]),
        5 => Some(&b"$-1\r\n"[..]),
        6 => Some(&b"*2\r\n:1\r\n:2\r\n"[..]),
        7 => Some(&b"*2\r\n*1\r\n+OK\r\n$3\r\nfoo\r\n"[..]),
        8 => Some(&b":7\r\ntrailing"[..]),
        9 => Some(&b"+OK\r"[..]),
        10 => Some(&b"$3\r\nfo"[..]),
        11 => Some(&b"*2\r\n:1\r\n"[..]),
        12 => Some(&b"$3\r\nfooXX"[..]),
        13 => Some(&b":not-int\r\n"[..]),
        _ => None,
    };
    if let Some(fixture) = fixture {
        return (fixture.to_vec(), false);
    }

    let len = (next_u64(state) as usize) % (MAX_INPUT_LEN + 1);
    let mut raw = vec![0_u8; len];
    for byte in &mut raw {
        *byte = next_u64(state) as u8;
    }
    (raw, true)
}

#[test]
fn bare_cr_before_crlf_is_line_content() {
    let expected = Value::simple_string(b"bad\rX");
    assert_eq!(Value::parse(b"+bad\rX\r\n").unwrap(), (expected.clone(), 8));
    assert_eq!(
        Value::parse_bytes(Bytes::from_static(b"+bad\rX\r\n")).unwrap(),
        (expected, 8)
    );
}

#[test]
fn deterministic_resp2_parser_diagnostic() {
    let started = Instant::now();
    let options = ParseOptions::new()
        .max_bulk_string_len(MAX_INPUT_LEN)
        .max_collection_elements(4)
        .max_total_items(8)
        .max_depth(4);
    let mut state = 0x7265_7370_322d_6b61;
    let mut coverage = Coverage::default();

    for iteration in 0..ITERATIONS {
        let (mut owned, random) = input_for(iteration, &mut state);
        owned.truncate(MAX_INPUT_LEN);
        coverage.random += usize::from(random);
        let input = owned.as_slice();
        let copied = Value::parse_with_options(input, &options);
        let zero_copy = Value::parse_bytes_with_options(Bytes::copy_from_slice(input), &options);

        match (copied, zero_copy) {
            (Ok((left, left_consumed)), Ok((right, right_consumed))) => {
                coverage.success += 1;
                coverage.trailing += usize::from(left_consumed < input.len());
                assert!(left_consumed <= input.len());
                assert_eq!(left_consumed, right_consumed);
                assert_eq!(left, right);

                let mut encoded = vec![0_u8; left.encoded_len()];
                let written = left.encode(&mut encoded);
                let (roundtrip, consumed) = Value::parse_with_options(&encoded, &options).unwrap();
                assert_eq!(roundtrip, left);
                assert_eq!(consumed, written);
                coverage.roundtrip += 1;
            }
            (Err(left), Err(right)) => {
                if left.is_incomplete() {
                    coverage.incomplete += 1;
                } else {
                    coverage.error += 1;
                }
                assert_eq!(left, right);
            }
            (left, right) => panic!("parser outcome mismatch: {left:?} != {right:?}"),
        }
    }

    let elapsed = started.elapsed();
    assert_eq!(
        coverage.success + coverage.incomplete + coverage.error,
        ITERATIONS
    );
    assert_eq!(coverage.roundtrip, coverage.success);
    eprintln!(
        "RESP2_DIAGNOSTIC iterations={ITERATIONS} elapsed_us={} throughput_per_s={:.0} success={} incomplete={} error={} trailing={} roundtrip={} random={}",
        elapsed.as_micros(),
        ITERATIONS as f64 / elapsed.as_secs_f64(),
        coverage.success,
        coverage.incomplete,
        coverage.error,
        coverage.trailing,
        coverage.roundtrip,
        coverage.random,
    );
}

/// Split `input` into pieces at the given cut points (sorted, may repeat,
/// so a piece can be empty).
fn split_at(input: &[u8], cuts: &[usize]) -> Vec<Bytes> {
    let mut pieces = Vec::with_capacity(cuts.len() + 1);
    let mut start = 0;
    for &cut in cuts {
        pieces.push(Bytes::copy_from_slice(&input[start..cut]));
        start = cut;
    }
    pieces.push(Bytes::copy_from_slice(&input[start..]));
    pieces
}

/// `parse_segments` on any split of an input gives what `parse_bytes` gives
/// on the whole input: the same value, consumed count and error.
#[test]
fn segmented_parse_matches_contiguous() {
    // A small line limit reaches the "line too long" path within 24 bytes.
    for options in [
        ParseOptions::new()
            .max_bulk_string_len(MAX_INPUT_LEN)
            .max_collection_elements(4)
            .max_total_items(8)
            .max_depth(4),
        ParseOptions::new().max_line_len(6),
    ] {
        let mut state = 0x7365_676d_656e_7473;
        let mut splits = 0usize;
        for iteration in 0..ITERATIONS {
            let (mut owned, _) = input_for(iteration, &mut state);
            owned.truncate(MAX_INPUT_LEN);
            let input = owned.as_slice();
            let whole = Value::parse_bytes_with_options(Bytes::copy_from_slice(input), &options);

            let mut check = |cuts: &[usize]| {
                let pieces = split_at(input, cuts);
                let segmented = Value::parse_segments_with_options(&pieces, &options);
                assert_eq!(
                    segmented, whole,
                    "input {input:?} split at {cuts:?} into {pieces:?}"
                );
                splits += 1;
            };

            // Every two-piece split, the empty ends included.
            for cut in 0..=input.len() {
                check(&[cut]);
            }
            // One byte per piece, past the eight pieces kept on the stack, and
            // the same with an empty piece between every two bytes.
            let every: Vec<usize> = (1..input.len()).collect();
            check(&every);
            let doubled: Vec<usize> = every.iter().flat_map(|&c| [c, c]).collect();
            check(&doubled);
            // Three random splits into three to five pieces, some empty.
            for _ in 0..3 {
                let n = 2 + (next_u64(&mut state) as usize) % 3;
                let mut cuts: Vec<usize> = (0..n)
                    .map(|_| (next_u64(&mut state) as usize) % (input.len() + 1))
                    .collect();
                cuts.sort_unstable();
                check(&cuts);
            }
        }
        assert!(splits > ITERATIONS * 4);
    }
}

/// `scan_segments` agrees with `parse_segments` on any split of an input:
/// complete with the same length, incomplete, or the same error.
#[test]
fn scan_agrees_with_parse() {
    for options in [
        ParseOptions::new()
            .max_bulk_string_len(MAX_INPUT_LEN)
            .max_collection_elements(4)
            .max_total_items(8)
            .max_depth(4),
        ParseOptions::new().max_line_len(6),
    ] {
        let mut state = 0x7363_616e_7365_6773;
        let mut seen = [0usize; 3];
        for iteration in 0..ITERATIONS {
            let (mut owned, _) = input_for(iteration, &mut state);
            owned.truncate(MAX_INPUT_LEN);
            let input = owned.as_slice();
            let mut check = |cuts: &[usize]| {
                let pieces = split_at(input, cuts);
                let parsed = Value::parse_segments_with_options(&pieces, &options);
                let scanned = Value::scan_segments_with_options(&pieces, &options);
                match (&parsed, &scanned) {
                    (Ok((_, n)), Ok(Frame::Complete(m))) if n == m => seen[0] += 1,
                    (Err(ParseError::Incomplete), Ok(Frame::Incomplete(need))) if *need > 0 => {
                        seen[1] += 1
                    }
                    (Err(e), Err(f)) if e == f => seen[2] += 1,
                    _ => panic!(
                        "input {input:?} split at {cuts:?}: parse {parsed:?}, scan {scanned:?}"
                    ),
                }
            };
            for cut in 0..=input.len() {
                check(&[cut]);
            }
            let every: Vec<usize> = (1..input.len()).collect();
            check(&every);
        }
        assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    }
}

#[test]
fn no_pieces_is_incomplete() {
    let parsed = Value::parse_segments(&[]);
    assert!(
        matches!(parsed, Err(ref e) if e.is_incomplete()),
        "{parsed:?}"
    );
}

/// A string that lies within one piece is a slice of that piece; one that
/// spans pieces is a copy.
#[test]
fn segmented_strings_are_zero_copy_within_a_piece() {
    let first = Bytes::from_static(b"*2\r\n$5\r\nhello\r\n$5\r\nwo");
    let second = Bytes::from_static(b"rld\r\n");
    let (value, consumed) = Value::parse_segments(&[first.clone(), second.clone()]).unwrap();
    assert_eq!(consumed, first.len() + second.len());
    let Value::Array(items) = value else {
        panic!("expected an array")
    };
    let (Value::BulkString(hello), Value::BulkString(world)) = (&items[0], &items[1]) else {
        panic!("expected two bulk strings")
    };
    assert_eq!(&hello[..], b"hello");
    assert_eq!(&world[..], b"world");
    let within = |b: &Bytes, piece: &Bytes| {
        let range = piece.as_ptr_range();
        range.contains(&b.as_ptr())
    };
    assert!(within(hello, &first), "hello lies in the first piece");
    assert!(
        !within(world, &first) && !within(world, &second),
        "world is copied"
    );
}

/// The line limit counts from where the unfinished line starts, not from the
/// start of the input: a nested element still waiting for its CRLF is
/// incomplete while its own bytes fit the limit.
#[test]
fn segmented_line_limit_counts_from_the_line_start() {
    let options = ParseOptions::new().max_line_len(6);
    let input = b"*2\r\n:1\r\n+abcd";
    let whole = Value::parse_bytes_with_options(Bytes::from_static(input), &options);
    assert!(
        matches!(whole, Err(ref e) if e.is_incomplete()),
        "{whole:?}"
    );
    for cut in 0..=input.len() {
        let pieces = split_at(input, &[cut]);
        assert_eq!(
            Value::parse_segments_with_options(&pieces, &options),
            whole,
            "split at {cut}"
        );
    }
}

/// RESP3 types: every prefix of each fixture, split at every point, parses
/// as the same prefix does contiguously.
#[cfg(feature = "resp3")]
#[test]
fn segmented_resp3_matches_contiguous() {
    let fixtures: &[&[u8]] = &[
        b"_\r\n",
        b"#t\r\n",
        b"#x\r\n",
        b",3.25\r\n",
        b",inf\r\n",
        b",nope\r\n",
        b"(12345678901234567890\r\n",
        b"!9\r\nERR wrong\r\n",
        b"=8\r\ntxt:abcd\r\n",
        b"=3\r\ntxt\r\n",
        b"=5\r\ntxtXy\r\n",
        b"%2\r\n+a\r\n:1\r\n+b\r\n#f\r\n",
        b"~3\r\n:1\r\n$2\r\nhi\r\n_\r\n",
        b">2\r\n+push\r\n=6\r\ntxt:ab\r\n",
        b"|1\r\n+ttl\r\n:5\r\n$3\r\nval\r\n",
        b"%1\r\n~1\r\n%1\r\n+k\r\n+v\r\n:0\r\n",
    ];
    let options = ParseOptions::new().max_depth(3);
    for fixture in fixtures {
        for end in 0..=fixture.len() {
            let input = &fixture[..end];
            let whole = Value::parse_bytes_with_options(Bytes::copy_from_slice(input), &options);
            // The copy parser is checked against the zero-copy one.
            assert_eq!(
                Value::parse_with_options(input, &options),
                whole,
                "input {input:?}"
            );
            for cut in 0..=input.len() {
                for cut2 in cut..=input.len() {
                    let pieces = split_at(input, &[cut, cut2]);
                    assert_eq!(
                        Value::parse_segments_with_options(&pieces, &options),
                        whole,
                        "input {input:?} split at {cut}, {cut2}"
                    );
                }
            }
        }
    }
}
