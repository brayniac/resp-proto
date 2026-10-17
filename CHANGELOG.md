# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `Value::parse_segments` and `Value::parse_segments_with_options` parse a
  value from bytes that arrived in several pieces (`&[Bytes]`), as a receive
  path that does not copy them into one buffer delivers them. The result is
  what `parse_bytes` gives on the pieces joined: the same value, consumed
  count and error. A string within one piece is a slice of it; a string that
  spans pieces is copied.
- `Value::scan_segments` and `Value::scan_segments_with_options` find the
  end of the value at the start of several pieces without building it:
  `Frame::Complete(len)` with the length `parse_segments` would consume,
  `Frame::Incomplete(n)` with a lower bound on the bytes still needed, or the
  error the parse would return. A caller can scan on each arrival and parse
  once, so an incomplete value's strings that span pieces are not copied and
  discarded on every arrival.

### Changed

- `parse_bytes` and `parse_segments` share one parser over positions in the
  input. Parsing an array or map no longer takes a `Bytes` slice of the rest
  of the input for each element. `parse_bytes` against the preceding commit
  (b52cc66) on arm64, medians of interleaved runs each the best of seven: a
  map of 20 entries about 30% faster, an array of 100 bulk strings about
  8-10% faster, a simple string about 6% faster, bulk strings of 64 B and
  16 KiB within 3%, and an integer about 1 ns (9%) slower.

## [0.0.2] - 2026-08-19

### Added

- `DEFAULT_MAX_LINE_LEN` (64 KiB, matching Redis's `PROTO_INLINE_MAX_SIZE`) and a
  `max_line_len` option on `ParseOptions`.

### Fixed

- RESP line framing no longer stalls on a bare `\r`. Both scanners -- `find_crlf`
  in the value parser and `Cursor::read_line` in the command parser -- inspected
  only the first `\r` in the buffer and gave up if it was not followed by `\n`.
  A complete, CRLF-terminated line containing a stray `\r` therefore reported
  `Incomplete` forever, and since neither path is bounded, more data could never
  help: the scan restarted at the same leading `\r` every time. Reachable in
  practice because Redis keys are binary-safe and error replies echo
  user-supplied arguments. Both now scan for a real CRLF.
- RESP line framing is now bounded. Both parsers reported `Incomplete` for an
  unterminated line no matter how large the buffer grew, so a peer that never
  sent a newline could make a caller buffer without limit. Both now return
  `Protocol("line too long")` past `max_line_len`. The bound applies to protocol
  lines only; bulk payload remains bounded separately by `max_bulk_string_len`.

## [0.0.1] - 2026-02-21

### Added

- Initial release extracted from ringline workspace
- RESP2 protocol parsing and encoding
- Optional RESP3 support via `resp3` feature flag
- Streaming parser for incremental decoding
- Cluster slot and shard response parsing
