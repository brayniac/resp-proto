//! Bounded proof for the pure RESP2 line-framing decision seam.
//!
//! `classify_first_cr` is called by the `memchr` scanner that `parse` and
//! `parse_bytes` use, and decides whether the first carriage return completes
//! a line. `parse_segments` scans with its own search across pieces, which
//! this proof does not cover. The proof covers every eight-byte input and
//! every optional candidate index. It does not prove the full parser or
//! `memchr` itself; runtime tests bridge the classifier contract to `parse`
//! and `parse_bytes`.

use crate::value::classify_first_cr;

const RAW_BYTES: usize = 8;

#[kani::proof]
fn first_cr_classifier_accepts_only_an_in_bounds_crlf() {
    let input: [u8; RAW_BYTES] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= RAW_BYTES);
    let data = &input[..len];
    let first_cr: Option<usize> = kani::any();
    if let Some(position) = first_cr {
        kani::assume(position < len);
        kani::assume(data[position] == b'\r');
    }

    let classified = classify_first_cr(data, first_cr);
    match classified {
        Some(position) => {
            assert_eq!(Some(position), first_cr);
            assert!(position + 1 < len);
            assert_eq!(data[position], b'\r');
            assert_eq!(data[position + 1], b'\n');
        }
        None => match first_cr {
            None => {}
            Some(position) => {
                assert!(position + 1 >= len || data[position + 1] != b'\n');
            }
        },
    }
}
