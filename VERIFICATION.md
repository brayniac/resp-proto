# RESP2 verification

This crate uses Kani for one bounded, production-used RESP2 framing decision and a deterministic diagnostic for broader parser behavior.

## Formal proof boundary

`classify_first_cr` decides whether a carriage return found by the production `memchr` path begins a complete CRLF. The scanner that `parse` and `parse_bytes` use applies it to successive candidates, so a bare `\r` is line content and scanning continues to the first real CRLF. `parse_segments` scans with its own search, which checks a carriage return at the end of one piece against the first byte of the next; the proof does not cover it, and the segmented differential tests below do. The same function is compiled in normal and Kani builds; there is no alternate proof-only implementation.

The proof checks every byte string of length 0 through 8 and every admissible candidate: `None`, or an in-bounds position containing CR. This is 19,097,521,942,299,935,490 constrained input/candidate combinations. It proves that a returned index is the supplied in-bounds CR immediately followed by LF, and that every rejected candidate is absent, truncated, or not followed by LF.

Run:

```console
cargo kani --harness first_cr_classifier_accepts_only_an_in_bounds_crlf --output-format=terse
```

With Kani 0.67.0, the proof completed in 0.292 seconds with 0 of 99 checks failed. Kani reported no unreachable or undetermined checks. Its compilation warning mentions unsupported foreign/caller-location constructs elsewhere in the compiled dependency graph; none is reachable in this harness, so verification succeeds.

This is not a full-parser proof. It does not prove `memchr`, integer conversion, allocation through `Bytes` or `Vec`, recursive array parsing, or the public zero-copy parsers (`parse_bytes`, `parse_segments`). Attempts to model the complete parser were disproportionate and encountered unsupported runtime SIMD/CPUID paths plus costly allocation models. Parser recursion is bounded in production by `ParseOptions::max_depth`; it is exercised by unit and deterministic integration tests, not this proof.

## Deterministic parser diagnostic

`tests/resp2_random.rs` runs the copy parser (`parse`) and the zero-copy parser (`parse_bytes`) over the same 100,000-input deterministic sequence. The corpus mixes simple strings, errors, integers, bulk strings, nulls, empty/nonempty/nested arrays, trailing data, realistic incomplete frames, malformed frames, and 12,500 fixed-seed raw random inputs. It requires identical values, consumed lengths, and complete `ParseError` values. Every success is encoded and parsed again.

This comparison is a regression diagnostic, not an independent oracle: the two parsers share the CRLF scanner and the null, boolean and double helpers, though the zero-copy parser reads integers with its own code. It can detect divergence in copy versus zero-copy paths and roundtrip regressions, but cannot establish full parser correctness by agreement alone.

Run:

```console
cargo test --release --test resp2_random deterministic_resp2_parser_diagnostic -- --nocapture
```

A reference run on 2026-08-19 produced:

| Iterations | Elapsed | Diagnostic throughput | Success | Incomplete | Error | Trailing | Roundtrip | Raw random |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 100,000 | 19.643 ms | 5,090,758/s | 56,250 | 19,495 | 24,255 | 6,250 | 56,250 | 12,500 |

Elapsed time and throughput are environment-dependent diagnostic context, not a performance baseline or benchmark claim. Semantic outcome counts and fixed iteration count are deterministic.

## Segmented parser

`parse_segments` parses a value split across several `Bytes` pieces. Five tests in `tests/resp2_random.rs` compare it with `parse_bytes` on the joined input, requiring the same value, consumed length and `ParseError`:

- `segmented_parse_matches_contiguous`: the same input generator as the diagnostic (100,000 inputs, a different seed), each split at every point, into three to five random pieces with some empty, into one byte per piece, and into one byte per piece with an empty piece between every two, with the diagnostic's limits and with a 6-byte line limit.
- `segmented_line_limit_counts_from_the_line_start`: a nested element whose unfinished line fits the limit is incomplete at every split.
- `segmented_resp3_matches_contiguous` (`resp3` feature): every RESP3 type, every prefix of each fixture, every split into three pieces; it also checks `parse` against `parse_bytes` on those inputs.
- `no_pieces_is_incomplete` and `segmented_strings_are_zero_copy_within_a_piece`.

Like the diagnostic, these compare parsers that share code; they detect divergence between the contiguous and segmented paths, not errors both make.
