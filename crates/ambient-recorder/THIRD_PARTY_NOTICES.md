Native codec dependencies built by `scripts/native-deps.sh`:

- Opus 1.6.1, Xiph.Org and contributors: [license](licenses/opus.txt).
- libopusenc 0.3, Xiph.Org and Jean-Marc Valin: [license](licenses/libopusenc.txt).
- libopusenc's bundled Speex resampler: [license](licenses/speex-resampler.txt).

Additional libopusenc notices cover its [Ogg packer](licenses/opusenc-ogg-packer.txt), [header](licenses/opusenc-header.txt), [picture support](licenses/opusenc-picture.txt) and [Unicode support](licenses/opusenc-unicode.txt). Preserve these notices with binary distributions, including when a recording path bypasses the native resampler.

Rust package versions and origins are recorded in `Cargo.lock`; their source distributions contain their respective license texts. The Rust implementation uses maintained Rubato resampling rather than the source application's Speex resampler; floating-point sample values can differ while acquired timestamps and output duration remain defined by this package.
