# CI toolchain drift after the Spec 010 push

The push of `095bb56` triggered Actions run `40514646404`. Linux build/tests
and both capability jobs passed. Windows build and format passed, then strict
Clippy stopped on `mash/src/lexer.rs`'s `drain(..).collect()` (`drain_collect`).
The runner used Rust 1.99; the local compiler reported 1.97. This is a newer
lint than the local toolchain's passing gate, not a passing hosted CI result.

The lexer now uses `std::mem::take`, preserving the drained collection's
contents without the extra allocation. The same log warned that atomic
`fetch_update` was deprecated. Client-slot admission now uses the equivalent
compare-exchange loop, keeping compatibility with the repository's older
Rust baseline instead of adopting the newly renamed API.

Local verification: helper admission test, shell lexer/executor tests, native
Windows Smoosh (183 passed, three unsupported skips via its runner), format,
and strict workspace Clippy passed. A new hosted run is required to establish
the repaired CI result.
