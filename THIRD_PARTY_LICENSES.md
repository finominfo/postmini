# Third-party components bundled in postmini

JavaScript libraries embedded in the script sandbox (`js/lib/`), minified, unmodified:

| Library | Version | License |
|---|---|---|
| chai | 4.x | MIT — Copyright (c) Jake Luer, chaijs |
| lodash | 4.17.21 | MIT — Copyright OpenJS Foundation and other contributors |
| crypto-js | 4.x | MIT — Copyright (c) 2009-2013 Jeff Mott, 2013-2016 Evan Vosberg |
| moment | 2.x | MIT — Copyright (c) JS Foundation and other contributors |
| ajv | 6.x | MIT — Copyright (c) 2015-2021 Evgeny Poberezkin |
| tv4 | 1.x | Public Domain / MIT |

Rust crates (see Cargo.lock for exact versions) are used under their respective MIT / Apache-2.0 / BSD / ISC /
MPL-2.0 licenses, among them: eframe/egui (MIT/Apache-2.0), rquickjs + QuickJS (MIT), rustls (Apache-2.0/MIT/ISC),
ring (ISC-style), webpki-roots (MPL-2.0 data), serde/serde_json (MIT/Apache-2.0), tungstenite (MIT/Apache-2.0),
cookie_store (MIT/Apache-2.0), flate2, brotli-decompressor (BSD-3/MIT), rfd (MIT), image (MIT/Apache-2.0).

Fonts bundled by egui: Ubuntu-Light (Ubuntu Font Licence), Hack (MIT/Bitstream Vera), Noto Emoji (OFL),
emoji-icon-font (MIT).

postmini is not affiliated with or endorsed by Postman, Inc. "Postman" is a trademark of Postman, Inc.
The collection/environment file formats are implemented for interoperability.
