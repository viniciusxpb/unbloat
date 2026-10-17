# Third-party software

Unbloat's own code and artwork are MIT licensed. Third-party source code remains
under its respective license. No 7-Zip binary, logo or artwork is distributed.

The release includes sevenz-rust2 0.23.0 (Apache-2.0), lzma-rust2 0.21.0
(Apache-2.0), ignore (MIT OR Unlicense), windows/windows-core (MIT OR Apache-2.0),
serde (MIT OR Apache-2.0), and toml (MIT OR Apache-2.0), plus their transitive
dependencies. The exact locked dependency inventory and original license/notice
texts are generated into this directory by `scripts/collect-licenses.ps1`.
These generated files are excluded from Git. Repository packaging and development
installation regenerate them automatically; binary distributions include them.

For dual-licensed MIT/Apache dependencies this distribution elects MIT. For
MIT/Unlicense dependencies it elects MIT. Apache-only dependencies retain the
Apache License 2.0 and all shipped notices. See `DEPENDENCIES.md` and the
`source-notices/` directory. The inventory includes build and target-specific
dependencies from Cargo.lock; not all of them are linked into the Windows binary.

No encryption, extra compression codecs, async runtime, external compressor,
telemetry or network client is enabled in the product.
