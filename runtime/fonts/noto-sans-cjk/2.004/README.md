# Noto Sans CJK Regular 2.004

Rocket embeds this unmodified font collection in **Linux UI builds only**. It is
the last fallback for code, interface text and terminal-wide characters. Installed
fonts retain priority, and the existing Latin fonts retain their metrics. The
collection supplies Chinese, Japanese and Korean forms, including monospace faces.
No font installation or runtime download is required. The two faces borrow one
immutable copy of the font bytes in each UI executable.

- Publisher: [Noto](https://github.com/notofonts/noto-cjk).
- [Release Sans2.004](https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004).
- Pinned commit: `523d033d6cb47f4a80c58a35753646f5c3608a78`.
- Upstream path: `Sans/OTC/NotoSansCJK-Regular.ttc`.
- Size: **19,484,784 bytes**.
- Publisher Git blob SHA-1: `a2033d0e4e53f568c3f418a7d5d8c951af3f76c1`.
- Recorded SHA-256: `b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a`.
- License: **SIL Open Font License 1.1**; unmodified upstream `LICENSE` is beside
  the font. `NOTICE` adds the copyright copied from the font's name table and is
  included by the notices generator in About → Legal and release documentation.
- Upstream LICENSE Git blob: `d952d62c065f3f35fb83a173496e90b21525aef3`;
  SHA-256: `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2`.

The download used the commit-pinned official HTTPS raw URLs, with redirects
disabled. Both Git object hashes and sizes matched publisher metadata. Upstream
does not publish a SHA-256 or signed release commit for this file; the user
explicitly approved Git-content-hash verification and recording SHA-256 instead.
The user reported a completed Malwarebytes scan with no threats on 2026-11-08,
before the font was loaded; Defender was disabled. This is bounded provenance
and scanning evidence, not a guarantee against undisclosed vulnerabilities.

On 2026-11-08, the font still matched upstream `main`, Sans2.004 remained the
latest listed Noto Sans CJK release, and the upstream security page listed no
published advisories. OSV returned no listed advisories for Rocket's locked
font-types 0.12.6, read-fonts 0.41.0, skrifa 0.44.0, harfrust 0.12.0 and epaint
0.36.2. These findings can change; recheck them before upgrading.
