The executable statically links native codec dependencies prepared by the sibling library:

- Opus 1.6.1, Xiph.Org and contributors: [license](licenses/opus.txt).
- libopusenc 0.3, Xiph.Org and Jean-Marc Valin: [license](licenses/libopusenc.txt).
- libopusenc's bundled Speex resampler: [license](licenses/speex-resampler.txt).

Additional libopusenc notices cover its [Ogg packer](licenses/opusenc-ogg-packer.txt), [header](licenses/opusenc-header.txt), [picture support](licenses/opusenc-picture.txt) and [Unicode support](licenses/opusenc-unicode.txt). Preserve these notices with binary distributions, including when recording bypasses the native resampler.

Rust dependency versions and origins are recorded in `Cargo.lock`; their source distributions contain their respective licenses. The sibling library uses Rubato resampling in place of the source application's Speex resampler; floating-point samples can differ while acquisition timestamps and output duration remain defined by the library.
