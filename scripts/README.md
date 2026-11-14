# `scripts/`

Generators and checks that produce files the release publishes. Nothing here
runs at runtime: the binary and the widget never read these.

## Files

- `third_party_notices.py` — writes `../THIRD_PARTY_NOTICES.md` from the crates
  linked into the release binary. Reads `cargo metadata` and `cargo tree` for
  the dependency set, and the licence files in the local cargo cache for the
  texts. `--check` compares the file on disk against what it would generate and
  exits 1 when they differ, which is how CI notices a dependency change that
  nobody regenerated.

  Scope is the normal dependencies of `x86_64-unknown-linux-musl`, **not**
  everything in `Cargo.lock`. The lock file includes `r-efi` (LGPL) and
  `termina` (MPL), which build only for other targets: listing them would
  describe a binary that does not exist, and in a licensing document an
  over-broad claim is a wrong claim.

  Finding a licence text needs `TEXT_MARKERS`, a phrase per licence, because
  neither filenames nor the SPDX id are reliable. `foldhash` ships Zlib as a
  bare `LICENSE`, and `ryu` ships BSL-1.0 as `LICENSE-BOOST`; neither file
  contains its own SPDX id.

## Usage

```bash
python3 scripts/third_party_notices.py            # regenerate
python3 scripts/third_party_notices.py --check     # CI gate
```

Regenerate whenever `Cargo.lock` changes and commit the result. `THIRD_PARTY_NOTICES.md`
is published by `release.yml`, and `tests/published_readme.rs` asserts every
published file also gets a digest in `bundle.json`.
