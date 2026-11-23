# postmini

Könnyűsúlyú, Postman-kompatibilis API kliens Rustban. Egyetlen ~13 MB-os `.exe`, telepítés és futtatókörnyezet nélkül.
A Postman collection / environment fájlokat (v2.1) közvetlenül olvassa és írja, a szkripteket (`pm.*` API, chai, lodash, CryptoJS, moment, ajv, tv4) beágyazott QuickJS motorban futtatja.

```
bin\postmini.exe          grafikus alkalmazás
bin\postmini-cli.exe      parancssori futtató (newman-kompatibilis), CI-hoz
```

## Hogyan készült

A Postman telepítőjét (dl.pstmn.io) a fejlesztői környezet hálózati szabályai blokkolták, ezért a működést a Postman
**saját, nyílt forrású futtatómotorjából** elemeztem — ugyanez a kód fut a desktop appban és a newmanben is:
`postman-runtime 7.57`, `postman-sandbox 6.9`, `postman-collection 5.3`, `newman 6.2`, `chai-postman`.
Innen jön pl. a változó-feloldás sorrendje (local → data → environment → collection → globals, max. 19 beágyazott
feloldás), a szkriptek futási sorrendje (collection → mappa → kérés), az auth-öröklés, a `protocolProfileBehavior`
beállítások, a body-pruning szabály, a dinamikus `{{$...}}` változók listája és a `pm` API felülete.

A viselkedést a valódi newmannel vetettem össze ugyanazon a teszt-collectionön (`examples/`): az eredmények
egyeznek; eltérés csak ott van, ahol a newman 6 sandboxa még nem tud valamit (top-level `await`), amit a Postman
app 11 és a postmini igen.

## Erőforrás-használat

Mérések Linuxon (ugyanaz a kód, mint a Windows build), lokális teszt szerver ellen:

| Forgatókönyv | Postman / newman | postmini | Arány |
|---|---|---|---|
| CLI: 41 kérés, 85 assertion, chai/ajv/lodash/CryptoJS/moment | newman: 158 MB RAM, 1,35 s CPU | 15 MB RAM, 0,16 s CPU | **10,5× RAM, 8,4× CPU** |
| CLI: 40 kérés, mindegyiken `pm.test` | newman: 150 MB, 1,08 s CPU | 10,6 MB, 0,03 s CPU | **14× RAM, 36× CPU** |
| CLI: 40 kérés szkript nélkül | newman: 157 MB, 0,79 s CPU | 7,9 MB, <0,01 s CPU | **20× RAM, >80× CPU** |
| GUI üresjáratban | Postman desktop: ~280 MB induláskor, 450–600 MB néhány collectionnel (publikus mérések) | saját heap ~27 MB, **0% CPU** (csak eseményre rajzol) | |
| GUI, 5,25 MB-os JSON válasz (630 000 sor) megjelenítve | | +24 MB, UI frame-idő átlag 0,26 ms | |

Megjegyzés a GUI-ról: a mérőkörnyezetben nem volt GPU, ezért ott a szoftveres OpenGL (Mesa llvmpipe) további ~67 MB-ot
és a görgetéskori CPU-t vitte. Valódi Windowsos gépen a GPU-driver ennél kevesebbet foglal; a Windows Feladatkezelőben
mért értéket a gépeden érdemes megnézni (Postmannel egymás mellett).

Miért ilyen kicsi: nincs Electron/Chromium; az egui reaktív módban csak eseményre rajzol; a JS motor csak akkor jön
létre, ha van szkript, a beágyazott könyvtárak (chai stb.) csak első használatkor töltődnek be; nagy válaszoknál csak a
látható sorok kerülnek layoutra; a JSON pretty-print fa-építés nélkül, streamelve formáz.

## Funkciók

**Kérések**
- Bármely HTTP metódus (egyedi is), URL-ben `{{változó}}` kiemelés (zöld = definiált, piros = nincs), hover-re a feloldott érték és forrása
- Query paraméterek ↔ URL kétirányú szinkron, path változók (`/users/:id`), leírások, Bulk Edit
- Body: none, form-data (szöveg + fájl, több fájl, content-type), x-www-form-urlencoded, raw (Text/JSON/XML/HTML/JavaScript + Beautify), binary, GraphQL (query + variables)
- cURL beillesztése közvetlenül az URL mezőbe → automatikus import (bash és Windows cmd „Copy as cURL” formátum is)
- Kérésenkénti beállítások (protocolProfileBehavior): SSL ellenőrzés, redirect követés, eredeti metódus megtartása, Authorization megtartása redirectnél, Referer eltávolítása, body GET-hez, URL-kódolás kikapcsolása, cookie jar kikapcsolása, max. redirect
- Automatikus fejlécek (User-Agent, Accept, Accept-Encoding, Cache-Control, Postman-Token, Host, Content-Length) megjeleníthetők
- Dokumentáció (leírás) kérésekhez, mappákhoz, collectionökhöz

**Authorization** (öröklés collection → mappa → kérés): No Auth, Inherit, Basic, Bearer, API Key (header/query), JWT Bearer (HS256/384/512), Digest (automatikus challenge-response, MD5/SHA-256/-sess, qop), OAuth 1.0 (HMAC-SHA1/256/512, PLAINTEXT, header/body/URL), OAuth 2.0 (token kézzel vagy „Get New Access Token”: Authorization Code, PKCE, Implicit, Password, Client Credentials, Refresh; böngészős bejelentkezés lokális callbackkel), Hawk, AWS Signature v4 (header és presigned URL), Akamai EdgeGrid

**Válasz**
- Pretty (JSON/XML/HTML formázva, szintaxis-kiemeléssel), Raw, Preview (képek megjelenítése, HTML szövegként), soronkénti szűrés, sortörés, másolás
- Státusz, idő (DNS / TCP / TLS / TTFB / letöltés bontásban), méret (fejléc/body, tömörített/kicsomagolt), távoli/helyi cím, TLS verzió és cipher
- Cookies, Headers, Test Results (All/Passed/Skipped/Failed), Timeline (elküldött fejlécek, redirect lánc), Visualize (template + adat)
- Mentés példaként (example), fájlba, vágólapra

**Szkriptek** (Pre-request és Tests, collection/mappa/kérés szinten, Postman-sorrendben)
- `pm.environment / globals / collectionVariables / variables / iterationData` (get/set/unset/has/clear/toObject/replaceIn)
- `pm.request` módosítható (headers add/upsert/remove, url.query, body, auth), `pm.response` (code, status, headers, json(), text(), responseTime, responseSize, cookies)
- `pm.test` (szinkron, async/Promise, `done` callback), `pm.test.skip`, `pm.expect` (valódi chai 4), `pm.response.to.have.status/header/body/jsonBody/jsonSchema`, `.to.be.ok/success/clientError/serverError/notFound/json/withBody` …
- `pm.sendRequest` (callback és `await`), `pm.cookies` + `pm.cookies.jar()`, `pm.info`, `pm.execution.setNextRequest/skipRequest/location`, `pm.visualizer.set`
- Top-level `await`, `setTimeout/setInterval`, `console.log/info/warn/error`
- `require('lodash' | 'chai' | 'crypto-js' | 'moment' | 'ajv' | 'tv4' | 'uuid' | 'xml2js' | 'csv-parse/lib/sync' | 'atob' | 'btoa')`, globális `_`, `CryptoJS`, `tv4`, `moment`, `xml2Json`, `Buffer`
- Régi (legacy) API: `tests[...]`, `postman.setEnvironmentVariable…`, `responseBody`, `responseCode`, `responseHeaders`, `responseTime`, `environment`, `globals`, `data`
- Snippet lista a szerkesztő mellett (mint a Postmanben), szkript-timeout és memórialimit

**Változók**: environmentek (default/secret típus, bulk edit, export/import), globals, collection változók, adatfájl (CSV/JSON) változók, lokális változók, 100+ dinamikus változó (`{{$guid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$randomInt}}`, `{{$randomEmail}}`, `{{$randomFullName}}` …), környezet gyorsnézet (👁) szerkeszthető értékekkel

**Collectionök**: fa nézet, drag & drop átrendezés (mappába / elé), jobb-klikk menü (hozzáadás, átnevezés, duplikálás, törlés, mozgatás, export, futtatás, mock), szűrés, mappa- és collection-beállítások (auth, szkriptek, változók, leírás, viselkedés), mentett példák (examples)

**Collection Runner**: kérések kiválasztása és sorrendje, iterációk, késleltetés, adatfájl (CSV/JSON), `setNextRequest` folyamvezérlés, leállítás első hibánál, változók perzisztálása, élő eredmények, szűrés, export

**CLI (newman-kompatibilis)**: `postmini-cli run collection.json -e env.json -g globals.json -n 3 -d data.csv --folder X --env-var k=v --global-var k=v --delay-request 100 --timeout-request 5000 -k --bail --reporter-json-export out.json --reporter-junit-export junit.xml --export-environment env-out.json --export-globals … --export-collection … --ssl-client-cert … --ssl-client-key … --ssl-extra-ca-certs … --proxy …` (kilépési kód 1 hiba esetén)

**Egyéb**
- Import: Postman collection v1 / v2.0 / v2.1, environment, globals, OpenAPI 3 / Swagger 2 (JSON és YAML; mappák tagek szerint, példa body-k sémából, auth, példa válaszok), cURL, fájlból, szövegből, URL-ről, drag & drop
- Export: collection v2.1, environment, globals, runner eredmények
- Kódgenerálás 19 nyelvre: cURL, cURL (Windows cmd), PowerShell, HTTPie, wget, nyers HTTP, JS fetch, Node axios, Python requests, Python http.client, Go, Java OkHttp, C# HttpClient, PHP cURL, Ruby, Rust reqwest, Swift, Kotlin, Dart
- History (dátum szerint, szűrés, mentés collectionbe), több fül (nem mentett jelzés, középső klikk zárás, fülek megmaradnak újraindításkor)
- Console: hálózati napló (kérés/válasz fejlécek, body), szkript logok, hibák
- Cookie-kezelő (domainenként, hozzáadás, törlés), cookie jar perzisztens
- WebSocket kliens (fejlécek, üzenetküldés, ping, üzenetlista szűréssel, wss is)
- Lokális mock szerver a mentett példákból (`x-mock-response-name`, `x-mock-response-code`, path változók, késleltetés, CORS, napló)
- Beállítások: timeout, SSL ellenőrzés, redirectek, max. válaszméret, User-Agent, proxy (nincs / rendszer: env + Windows Internet Settings / egyedi HTTP vagy SOCKS5, auth, bypass lista), CA tanúsítványok, kliens tanúsítványok (PEM, host szerint), téma (sötét/világos), zoom, betűméret, munkakönyvtár a relatív fájlútvonalakhoz
- Gyorsbillentyűk: Ctrl+Enter küldés, Ctrl+S mentés, Ctrl+Shift+S mentés másként, Ctrl+T/N új fül, Ctrl+W bezárás, Ctrl+Tab váltás, Ctrl+O import, Ctrl+, beállítások, Ctrl+B oldalsáv, Ctrl+Alt+C console, Ctrl +/−/0 zoom

## Ami nincs benne (és miért)

- Felhőfunkciók: fiók, csapat-munkaterületek, szinkron, API Builder, Monitors, Flows, Postbot (AI), publikus dokumentáció — ezek a Postman szervereihez kötöttek. A postmini mindent helyben tárol, sima JSON fájlokban (git-tel verziózható).
- HTTP/2, gRPC, MQTT, Socket.IO, GraphQL séma-lekérdezés/autocomplete
- NTLM és ASAP auth, JWT RS*/ES* algoritmusok (a mezők megmaradnak a kompatibilitás miatt)
- `pm.visualizer` HTML-renderelése (a template és az adat látszik), HTML preview böngészőmotorral (szövegként jelenik meg)
- `pm.require` csomagkönyvtár, Postman Vault, `pm.execution.runRequest`
- Initial / current érték szétválasztása a változóknál (egy érték van, mint a v2.1 fájlformátumban)
- Interceptor / proxy-alapú forgalomrögzítés

## Adatok helye

`%APPDATA%\postmini` (collectionök: `collections\<id>.postman_collection.json`, environmentek, globals, history, cookie-k, beállítások).
Hordozható mód: hozz létre egy `postmini-data` mappát a `postmini.exe` mellett — ekkor oda ment. Más hely: `POSTMINI_HOME` környezeti változó.

## Fordítás

Windows, natívan (Rust + Visual Studio Build Tools a QuickJS és ring C kódjához):
```
cargo build --release
copy target\release\postmini.exe bin\
cargo build --release --features console --target-dir target-cli
copy target-cli\release\postmini.exe bin\postmini-cli.exe
```
Linuxról Windowsra (mingw-w64): `cargo build --release --target x86_64-pc-windows-gnu --config 'target.x86_64-pc-windows-gnu.linker="x86_64-w64-mingw32-gcc"'`.
Tesztek: `cargo test` (a `ws_echo` teszt `--ignored`, futó WebSocket echo szervert igényel).

## Példák / tesztelés

`examples/` — `selftest.postman_collection.json` (20 kérés, 40+ assertion: szkriptek, auth, cookie, redirect, gzip,
chunked, multipart, GraphQL, XML, sendRequest, setNextRequest …), `env.json`, `data.csv`, `upload.txt`,
`petstore.yaml` (OpenAPI), és `test_server.py` (postman-echo-szerű lokális szerver, HTTP 8765 + HTTPS 8766).

```
python examples\test_server.py
bin\postmini-cli.exe run examples\selftest.postman_collection.json -e examples\env.json -d examples\data.csv -k --working-dir examples
```
Az „Expected failure” teszt szándékosan bukik.

## Projektszerkezet

```
src/model.rs      Postman Collection v2.1 adatmodell (ismeretlen mezőket megőrzi)
src/vars.rs       változó-scope-ok, {{}} feloldás, dinamikus változók
src/http.rs       saját HTTP/1.1 kliens (rustls, proxy, SOCKS5, redirect, cookie, gzip/deflate/br, időzítések)
src/auth.rs       összes auth + OAuth2 token lekérés
src/prepare.rs    kérés előkészítése (body módok, multipart, alapértelmezett fejlécek)
src/script.rs     QuickJS sandbox, újrahasznosított engine
js/prelude.js     a pm API JavaScript implementációja
js/lib/           beágyazott chai, lodash, crypto-js, moment, ajv, tv4 (MIT)
src/runner.rs     kérés-életciklus és Collection Runner
src/cli.rs        newman-kompatibilis CLI (JSON / JUnit riport)
src/importer.rs   cURL, Postman v1, OpenAPI/Swagger import
src/codegen.rs    kódgenerálás
src/mock.rs       lokális mock szerver
src/ws.rs         WebSocket kliens
src/storage.rs    munkaterület tárolása
src/ui/           egui felület
```
