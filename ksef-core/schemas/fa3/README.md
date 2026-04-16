# FA(3) official XSD bundle

This directory vendors the official FA(3) schema bundle used by `ksef-core`.

Current pinned variant:
- CRD namespace: `http://crd.gov.pl/wzor/2025/06/25/13775/`
- Main schema URL: `http://crd.gov.pl/wzor/2025/06/25/13775/schemat.xsd`
- Download date: `2026-04-16`

Bundled files:
- `2025-06-25-13775/schemat.xsd`
- `2025-06-25-13775/StrukturyDanych_v10-0E.xsd`
- `2025-06-25-13775/ElementarneTypyDanych_v10-0E.xsd`
- `2025-06-25-13775/KodyKrajow_v10-0E.xsd`
- `2025-06-25-13775/wyroznik.xml`

Upstream source URLs:
- `http://crd.gov.pl/wzor/2025/06/25/13775/schemat.xsd`
- `http://crd.gov.pl/wzor/2025/06/25/13775/wyroznik.xml`
- `http://crd.gov.pl/xml/schematy/dziedzinowe/mf/2022/01/05/eD/DefinicjeTypy/StrukturyDanych_v10-0E.xsd`
- `http://crd.gov.pl/xml/schematy/dziedzinowe/mf/2022/01/05/eD/DefinicjeTypy/ElementarneTypyDanych_v10-0E.xsd`
- `http://crd.gov.pl/xml/schematy/dziedzinowe/mf/2022/01/05/eD/DefinicjeTypy/KodyKrajow_v10-0E.xsd`

Notes:
- `schemaLocation` values were rewritten from absolute CRD URLs to local filenames
  so validation is deterministic and works offline in tests.
- `UPSTREAM_SHA256SUMS.txt` stores checksums of raw files exactly as downloaded.
- `SHA256SUMS.txt` stores checksums of the vendored local files used by tests.
- Rust bindings regeneration command:
  `KSEF_FA3_SKIP_BINDINGS_CHECK=1 cargo run -p ksef-core --example generate_fa3_types`

## HOWTO: Update schema

1. Create/update schema version directory, for example:
   `ksef-core/schemas/fa3/2026-xx-xx-xxxxx/`.
2. Download raw upstream files from CRD into that directory:
   - `schemat.xsd`
   - `StrukturyDanych_v10-0E.xsd`
   - `ElementarneTypyDanych_v10-0E.xsd`
   - `KodyKrajow_v10-0E.xsd`
   - `wyroznik.xml`
3. Save raw checksums (exactly as downloaded):
   `sha256sum schemat.xsd StrukturyDanych_v10-0E.xsd ElementarneTypyDanych_v10-0E.xsd KodyKrajow_v10-0E.xsd wyroznik.xml > UPSTREAM_SHA256SUMS.txt`
4. Rewrite `schemaLocation` references to local filenames (offline deterministic validation).
5. Save vendored checksums (after local rewrite):
   `sha256sum schemat.xsd StrukturyDanych_v10-0E.xsd ElementarneTypyDanych_v10-0E.xsd KodyKrajow_v10-0E.xsd wyroznik.xml > SHA256SUMS.txt`
6. Regenerate Rust types:
   `KSEF_FA3_SKIP_BINDINGS_CHECK=1 cargo run -p ksef-core --example generate_fa3_types`
7. Run validation/tests:
   - `cargo test -p ksef-core --test fa3_xsd_validation`
   - `cargo test -p ksef-core --no-fail-fast`
8. Update this README (`Current pinned variant`, URLs, download date).

## What Is Modified Locally

Only `schemaLocation` paths are rewritten from CRD URLs to local filenames.
This means:
- `UPSTREAM_SHA256SUMS.txt` and `SHA256SUMS.txt` can differ for modified files.
- differences are expected and intentional.

## What Is Auto-Detected vs Not

- Auto-detected now:
  - stale generated Rust bindings vs local vendored XSD (`build.rs` fails build and tells to run generator).
  - local file drift (you can verify with `sha256sum -c SHA256SUMS.txt`).
- Not auto-detected now:
  - that CRD published a newer schema version upstream.

To detect new upstream versions, you still need a periodic manual check of CRD URLs
or add a scheduled CI job that downloads upstream and compares checksums.
