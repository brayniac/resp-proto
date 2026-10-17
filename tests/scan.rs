//! `Value::scan_segments` on prefixes of valid values: a prefix is
//! incomplete with a need no larger than the bytes missing, and the whole
//! value is complete at its length, with or without bytes after it.

use bytes::Bytes;
use resp_proto::{Frame, ParseOptions, Value};

fn next_u64(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 11
}

fn bytes_of(state: &mut u64, max: usize) -> Vec<u8> {
    let len = (next_u64(state) as usize) % (max + 1);
    (0..len)
        .map(|_| b"abcdefgh\r\n"[(next_u64(state) % 10) as usize])
        .collect()
}

/// A random value nested at most `depth` levels.
fn value_of(state: &mut u64, depth: usize) -> Value {
    let kinds = if cfg!(feature = "resp3") { 15 } else { 6 };
    let kind = next_u64(state) % if depth == 0 { 5 } else { kinds };
    let empty = next_u64(state) & 1 == 0;
    let mut items = |n: u64| -> Vec<Value> {
        (0..next_u64(state) % n)
            .map(|_| value_of(state, depth.saturating_sub(1)))
            .collect()
    };
    match kind {
        // An empty simple string, `+\r\n`, is the shortest value.
        0 if empty => Value::simple_string(b""),
        0 => Value::simple_string(b"OK"),
        1 => Value::error(b"ERR boom"),
        2 => Value::integer(next_u64(state) as i64 - (1 << 40)),
        3 => Value::bulk_string(&bytes_of(state, 40)),
        4 => Value::null(),
        5 => Value::array(items(4)),
        #[cfg(feature = "resp3")]
        6 => Value::boolean(next_u64(state) & 1 == 0),
        #[cfg(feature = "resp3")]
        7 => Value::double(1.5),
        #[cfg(feature = "resp3")]
        8 => Value::big_number(b"123456789012345678901234567890"),
        #[cfg(feature = "resp3")]
        9 => Value::bulk_error(&bytes_of(state, 20)),
        #[cfg(feature = "resp3")]
        10 => Value::verbatim_string(*b"txt", &bytes_of(state, 20)),
        #[cfg(feature = "resp3")]
        11 => {
            let keys = items(3);
            let values = items(3);
            Value::map(keys.into_iter().zip(values).collect())
        }
        #[cfg(feature = "resp3")]
        12 => Value::set(items(4)),
        #[cfg(feature = "resp3")]
        13 => {
            let attrs = items(2);
            let values = items(2);
            Value::attribute(
                attrs.into_iter().zip(values).collect(),
                value_of(state, depth.saturating_sub(1)),
            )
        }
        #[cfg(feature = "resp3")]
        14 => Value::push(items(4)),
        _ => Value::array(items(4)),
    }
}

fn encoded(value: &Value) -> Vec<u8> {
    let mut buf = vec![0u8; value.encoded_len()];
    let n = value.encode(&mut buf);
    buf.truncate(n);
    buf
}

/// `input` in pieces cut at random points, some empty.
fn pieces_of(input: &[u8], state: &mut u64) -> Vec<Bytes> {
    let n = (next_u64(state) % 5) as usize;
    let mut cuts: Vec<usize> = (0..n)
        .map(|_| (next_u64(state) as usize) % (input.len() + 1))
        .collect();
    cuts.sort_unstable();
    let mut pieces = Vec::new();
    let mut start = 0;
    for cut in cuts {
        pieces.push(Bytes::copy_from_slice(&input[start..cut]));
        start = cut;
    }
    pieces.push(Bytes::copy_from_slice(&input[start..]));
    pieces
}

#[test]
fn every_prefix_needs_at_most_the_missing_bytes() {
    let options = ParseOptions::new()
        .max_collection_elements(16)
        .max_total_items(1024);
    let mut state = 0x6e65_6564_6564;
    let mut prefixes = 0usize;
    for _ in 0..5_000 {
        let value = value_of(&mut state, 3);
        let whole = encoded(&value);
        for len in 0..whole.len() {
            let pieces = pieces_of(&whole[..len], &mut state);
            match Value::scan_segments_with_options(&pieces, &options) {
                Ok(Frame::Incomplete(need)) => assert!(
                    need >= 1 && need <= whole.len() - len,
                    "{value:?}: {len} of {} bytes, need {need}",
                    whole.len()
                ),
                other => panic!("{value:?}: {len} of {} bytes: {other:?}", whole.len()),
            }
            prefixes += 1;
        }
        let mut more = whole.clone();
        more.extend_from_slice(b"+next\r\n");
        for input in [&whole[..], &more[..]] {
            let pieces = pieces_of(input, &mut state);
            assert_eq!(
                Value::scan_segments_with_options(&pieces, &options),
                Ok(Frame::Complete(whole.len())),
                "{value:?}"
            );
            let (parsed, consumed) = Value::parse_segments_with_options(&pieces, &options).unwrap();
            assert_eq!((parsed, consumed), (value.clone(), whole.len()));
        }
    }
    assert!(prefixes > 50_000);
}

/// A bulk string's need is exact once its header is in: the body, its CRLF,
/// and nothing for values that follow.
#[test]
fn a_bulk_string_needs_its_body() {
    let options = ParseOptions::new();
    let pieces = [Bytes::from_static(b"$10\r\nab"), Bytes::from_static(b"cd")];
    assert_eq!(
        Value::scan_segments_with_options(&pieces, &options),
        Ok(Frame::Incomplete(8))
    );
    // Two elements still to come after a short bulk string: 3 bytes each at
    // least.
    let pieces = [Bytes::from_static(b"*3\r\n$4\r\nab")];
    assert_eq!(
        Value::scan_segments_with_options(&pieces, &options),
        Ok(Frame::Incomplete(4 + 6))
    );
}

/// Each error the parser returns, the scanner returns too, at every split.
#[test]
fn scan_reports_the_parsers_errors() {
    let strict = ParseOptions::new()
        .max_depth(2)
        .max_collection_elements(3)
        .max_total_items(4)
        .max_bulk_string_len(4)
        .max_line_len(16);
    let mut cases: Vec<&[u8]> = vec![
        b"*1\r\n*1\r\n*1\r\n+x\r\n",
        // Too deep before the header line is complete.
        b"*1\r\n*1\r\n*1",
        b"*4\r\n:1\r\n:2\r\n:3\r\n:4\r\n",
        b"*3\r\n*3\r\n:1\r\n",
        b"$5\r\nhello\r\n",
        b"$2\r\nabXY",
        b":12x\r\n",
        b"?bad\r\n",
        b"+this line is longer than sixteen bytes",
    ];
    if cfg!(feature = "resp3") {
        cases.extend([
            &b"%1\r\n%1\r\n%1\r\n+k\r\n+v\r\n+k\r\n+v\r\n"[..],
            b"~4\r\n:1\r\n:2\r\n:3\r\n:4\r\n",
            b"=4\r\ntxtx\r\n",
            b"_xx",
            b"#x\r\n",
            b",abc\r\n",
            b"!5\r\nboom!\r\n",
            b"|1\r\n|1\r\n|1\r\n+a\r\n+b\r\n",
            b"%1\r\n%1\r\n%1",
            b"~1\r\n~1\r\n~1",
        ]);
    }
    for input in cases {
        for cut in 0..=input.len() {
            let pieces = [
                Bytes::copy_from_slice(&input[..cut]),
                Bytes::copy_from_slice(&input[cut..]),
            ];
            let parsed = Value::parse_segments_with_options(&pieces, &strict);
            let scanned = Value::scan_segments_with_options(&pieces, &strict);
            let error = parsed.as_ref().expect_err("each case is an error");
            assert_eq!(
                scanned.as_ref().err(),
                Some(error),
                "{:?} cut at {cut}",
                String::from_utf8_lossy(input)
            );
        }
    }
}

/// RESP3 nulls, `_\r\n`, are the shortest values: an array of them needs 3
/// bytes per element still to come.
#[cfg(feature = "resp3")]
#[test]
fn every_prefix_of_an_array_of_nulls_needs_at_most_the_missing_bytes() {
    let options = ParseOptions::new();
    let whole = b"*3\r\n_\r\n_\r\n_\r\n";
    for len in 0..whole.len() {
        let pieces = [Bytes::copy_from_slice(&whole[..len])];
        match Value::scan_segments_with_options(&pieces, &options) {
            Ok(Frame::Incomplete(need)) => assert!(
                need >= 1 && need <= whole.len() - len,
                "{len} bytes, need {need}"
            ),
            other => panic!("{len} bytes: {other:?}"),
        }
    }
    let pieces = [Bytes::from_static(whole)];
    assert_eq!(
        Value::scan_segments_with_options(&pieces, &options),
        Ok(Frame::Complete(whole.len()))
    );
}
