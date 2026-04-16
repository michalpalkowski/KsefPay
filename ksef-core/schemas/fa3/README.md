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
