# Windows GNU packaging patch

Source: crates.io iroh 1.4.1 (MIT OR Apache-2.0), unmodified Rust sources.
Remove `cdylib` from normalized Cargo.toml crate-type. This application statically
links the Rust library and does not use the C dynamic-library artifact. Building
the unused DLL with Windows GNU fails with `export ordinal too large: 99756`.
The `lib` target remains; no networking or cryptographic behavior is changed.
